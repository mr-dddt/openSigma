mod agent;
mod config;
mod data;
mod execution;
mod journal;
mod signals;
mod telegram;
mod tui;
mod types;

use std::sync::Arc;

use anyhow::Result;
use crossterm::ExecutableCommand;
use tokio::sync::{mpsc, RwLock};
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::agent::llm_client::LlmClient;
use crate::agent::llm_gate::LlmGate;
use crate::agent::second_look::SecondLookScheduler;
use crate::agent::tuner::{SignalTuner, TuneTrigger};
use crate::config::{Config, Secrets};
use crate::data::hyperliquid::HyperliquidFeed;
use crate::data::news::NewsFeed;
use crate::data::polymarket::PolymarketFeed;
use crate::execution::hyperliquid::HlExecutor;
use crate::execution::kill_switch::KillSwitch;
use crate::execution::polymarket::PmExecutor;
use crate::execution::position_monitor::{PositionEvent, PositionMonitor};
use crate::execution::risk::RiskChecker;
use crate::journal::logger::TradeLogger;
use crate::journal::memory::MemoryManager;
use crate::journal::reporter::Reporter;
use crate::signals::aggregator::SignalAggregator;
use crate::signals::indicators::Indicators;
use crate::telegram::TelegramClient;
use crate::tui::app::{App, PerformanceStats, PositionInfo};
use crate::types::*;

const TUNE_SYSTEM_PROMPT: &str = r#"You are openSigma's signal engine tuner. Analyze the recent trade history and current signal parameters, then suggest parameter adjustments to improve performance.

Respond with ONLY a valid JSON object:
{"adjustments":[{"param":"<name>","old_value":<current>,"new_value":<suggested>}],"reasoning":"..."}

Tunable parameters:
- ema_cross_weight (int, weight for EMA 9/21 crossover signal)
- cvd_weight (int, weight for cumulative volume delta)
- rsi_weight (int, weight for RSI signal)
- ob_weight (int, weight for order book imbalance)
- stoch_rsi_weight (int, weight for stochastic RSI)
- strong_threshold (int, net score needed for STRONG signal)
- lean_threshold (int, net score needed for LEAN signal)
- rsi_oversold (float, RSI level considered oversold)
- rsi_overbought (float, RSI level considered overbought)
- min_atr_pct (float, minimum ATR% to allow trading)

Rules:
- Only suggest changes you are confident will improve performance
- Provide clear reasoning based on the trade data
- Keep adjustments small and conservative
- If no changes needed, return empty adjustments array"#;

enum TuiUpdate {
    Price(f64),
    Signal(SignalSnapshot),
    Log(String),
    Positions(Vec<PositionInfo>),
    Stats(PerformanceStats),
}

#[tokio::main]
async fn main() -> Result<()> {
    // Log to file to avoid interfering with TUI rendering
    std::fs::create_dir_all("data").ok();
    std::fs::create_dir_all("data/reports").ok();
    let log_file = std::fs::File::create("data/opensigma.log")?;
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "opensigma=info".into()),
        )
        .with_writer(log_file)
        .with_ansi(false)
        .init();

    info!("openSigma v1 starting...");

    // Load config (shared via Arc<RwLock> for tuner writes)
    let config = Config::load("config.toml")?;
    let secrets = Secrets::from_env()?;

    info!(
        capital = config.capital.initial_usd,
        max_trade_pct = config.capital.max_trade_pct,
        max_leverage = config.hyperliquid.max_leverage,
        "Config loaded"
    );

    let shared_config = Arc::new(RwLock::new(config.clone()));

    // Channel for market events (all data feeds → signal engine)
    let (market_tx, mut market_rx) = mpsc::channel::<MarketEvent>(2048);

    // Spawn data feeds
    let hl_feed = HyperliquidFeed::new(market_tx.clone());
    tokio::spawn(async move { hl_feed.run().await });

    let pm_feed = PolymarketFeed::new(market_tx.clone());
    tokio::spawn(async move { pm_feed.run().await });

    let mut news_feed = NewsFeed::new();
    tokio::spawn(async move { news_feed.run().await });

    // Initialize components
    let mut aggregator = SignalAggregator::new();
    let memory = Arc::new(MemoryManager::new("memory/memory.md"));
    let journal = TradeLogger::new("data/journal.jsonl");

    let llm_client = LlmClient::new(
        secrets.anthropic_api_key.clone(),
        config.llm.model.clone(),
        config.llm.timeout_ms,
    );
    let llm_gate = LlmGate::new(llm_client, memory.clone());

    // Separate LLM client for tuning (avoids borrow conflicts)
    let tune_client = LlmClient::new(
        secrets.anthropic_api_key.clone(),
        config.llm.model.clone(),
        config.llm.timeout_ms * 3, // longer timeout for tuning
    );

    // Reporter for 20-trade batch analysis
    let reporter = Reporter::new(&secrets.anthropic_api_key, &config.llm.model);

    // Telegram alerts (optional)
    let telegram = if config.telegram.enabled {
        TelegramClient::new(secrets.telegram_bot_token.clone(), secrets.telegram_chat_id.clone())
    } else {
        None
    };

    let mut second_look = SecondLookScheduler::new(config.execution.max_second_looks);
    let mut tuner = SignalTuner::new(&config);
    let mut risk = RiskChecker::new(config.capital.initial_usd);
    let mut position_monitor = PositionMonitor::new();
    let mut kill_switch = KillSwitch::new();
    let initial_usd = config.capital.initial_usd;

    let hl_executor = match HlExecutor::new(&secrets.hl_private_key).await {
        Ok(ex) => Some(ex),
        Err(e) => {
            warn!("HlExecutor init failed (trading disabled): {e:#}");
            None
        }
    };
    let pm_executor = PmExecutor::new(&secrets.pm_api_key, &secrets.pm_api_secret, &secrets.pm_passphrase);

    let eval_interval =
        tokio::time::Duration::from_secs(config.execution.signal_eval_interval_secs);
    let mut eval_ticker = tokio::time::interval(eval_interval);

    // TUI update channel
    let (tui_tx, mut tui_rx) = mpsc::channel::<TuiUpdate>(256);
    let tui_tx_clone = tui_tx.clone();
    let config_for_engine = shared_config.clone();

    // Spawn the market event processor + signal engine + LLM gate
    tokio::spawn(async move {
        loop {
            tokio::select! {
                Some(event) = market_rx.recv() => {
                    match &event {
                        MarketEvent::Price(tick) => {
                            aggregator.update_price(tick.price);
                            if tick.symbol == Symbol::BTC {
                                let _ = tui_tx_clone.send(TuiUpdate::Price(tick.price)).await;

                                // Check position price levels
                                // Software-side stop/TP monitoring. HL also has trigger orders
                                // for the same levels. remove_trade() guards against double-processing.
                                // TODO: after SDK integration, subscribe to HL order fills to detect
                                // exchange-side closes and cancel redundant trigger orders.
                                let events = position_monitor.check_price_levels(tick.price);
                                for pe in events {
                                    match pe {
                                        PositionEvent::StopHit(id) | PositionEvent::TakeProfitHit(id) => {
                                            let reason = match pe {
                                                PositionEvent::StopHit(_) => "stop_loss",
                                                PositionEvent::TakeProfitHit(_) => "take_profit",
                                                _ => "unknown",
                                            };
                                            if let Some(trade) = position_monitor.remove_trade(&id) {
                                                let leverage = trade.leverage.unwrap_or(1) as f64;
                                                let pnl = match trade.direction {
                                                    Direction::Long => (tick.price - trade.entry_price) / trade.entry_price * trade.size_usd * leverage,
                                                    Direction::Short => (trade.entry_price - tick.price) / trade.entry_price * trade.size_usd * leverage,
                                                };
                                                risk.record_trade_pnl(pnl);
                                                let pnl_pct = (risk.current_equity() - initial_usd) / initial_usd * 100.0;
                                                aggregator.update_daily_pnl(pnl_pct);
                                                tuner.record_trade();

                                                let record = TradeRecord {
                                                    id: trade.id,
                                                    ts_open: trade.opened_at,
                                                    ts_close: Some(chrono::Utc::now()),
                                                    duration_secs: Some((chrono::Utc::now() - trade.opened_at).num_seconds() as u64),
                                                    play_type: trade.play_type,
                                                    direction: trade.direction,
                                                    signal_level: trade.signal_level,
                                                    signal_score: trade.signal_score,
                                                    entry_price: trade.entry_price,
                                                    exit_price: Some(tick.price),
                                                    size_usd: trade.size_usd,
                                                    leverage: trade.leverage,
                                                    pnl_usd: Some(pnl),
                                                    exit_reason: Some(reason.to_string()),
                                                    llm_reasoning: trade.llm_reasoning.clone(),
                                                    capital_after: Some(risk.current_equity()),
                                                };
                                                let _ = journal.log_entry(&record);

                                                // Telegram alert on trade close
                                                if let Some(ref tg) = telegram {
                                                    let r = record.clone();
                                                    let tg_ref = tg.clone();
                                                    tokio::spawn(async move { tg_ref.send_trade_close(&r).await });
                                                }

                                                let _ = tui_tx_clone.send(TuiUpdate::Log(format!(
                                                    "[{}] {} closed ({}) PnL: ${:.2}",
                                                    chrono::Utc::now().format("%H:%M:%S"),
                                                    trade.direction, reason, pnl,
                                                ))).await;

                                                // Send updated stats
                                                let _ = tui_tx_clone.send(TuiUpdate::Stats(PerformanceStats {
                                                    total_trades: risk.total_closed(),
                                                    win_rate: risk.win_rate(),
                                                    total_pnl: risk.current_equity() - initial_usd,
                                                    streak: risk.streak(),
                                                })).await;
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                                risk.set_open_positions(position_monitor.open_count());

                                // Send position updates to TUI
                                let pos_infos: Vec<PositionInfo> = position_monitor.active_trades().iter().map(|t| {
                                    let pnl = match t.direction {
                                        Direction::Long => (tick.price - t.entry_price) / t.entry_price * t.size_usd * t.leverage.unwrap_or(1) as f64,
                                        Direction::Short => (t.entry_price - tick.price) / t.entry_price * t.size_usd * t.leverage.unwrap_or(1) as f64,
                                    };
                                    PositionInfo {
                                        id: t.id,
                                        direction: t.direction,
                                        play_type: t.play_type,
                                        entry_price: t.entry_price,
                                        size_usd: t.size_usd,
                                        leverage: t.leverage.unwrap_or(1),
                                        unrealized_pnl: pnl,
                                        duration_secs: (chrono::Utc::now() - t.opened_at).num_seconds().max(0) as u64,
                                    }
                                }).collect();
                                let _ = tui_tx_clone.send(TuiUpdate::Positions(pos_infos)).await;
                            }
                        }
                        MarketEvent::Trade(trade) => {
                            let is_buy = trade.side == Direction::Long;
                            aggregator.indicators.add_trade(trade.size, is_buy);
                            aggregator.push_trade_for_candles(trade.price, trade.size, trade.timestamp);
                        }
                        MarketEvent::OrderBook(book) => {
                            let imbalance = Indicators::ob_imbalance(&book.bids, &book.asks, 10);
                            aggregator.update_ob_imbalance(imbalance);
                        }
                        MarketEvent::Funding(tick) => {
                            aggregator.update_funding(tick.rate);
                        }
                        MarketEvent::Liquidation(_) => {}
                        MarketEvent::PmOdds(odds) => {
                            aggregator.update_pm_odds(odds.up_price);
                        }
                    }
                }
                _ = eval_ticker.tick() => {
                    if kill_switch.triggered {
                        continue;
                    }

                    let cfg = config_for_engine.read().await;
                    let snapshot = aggregator.evaluate(&cfg);
                    let _ = tui_tx_clone.send(TuiUpdate::Signal(snapshot.clone())).await;

                    // Check position expirations (max hold time)
                    let expired = position_monitor.check_expirations();
                    for id in expired {
                        if let Some(trade) = position_monitor.remove_trade(&id) {
                            info!(id = %trade.id, "Position expired (max hold time)");
                            // Close via executor if available
                            let current_price = aggregator.latest_price();
                            if let Some(ref hl) = hl_executor {
                                let is_buy = trade.direction == Direction::Short;
                                let sz = trade.size_usd / trade.entry_price;
                                let _ = hl.market_order("BTC", is_buy, sz, trade.leverage.unwrap_or(1)).await;
                            }
                            // Compute PnL for expired trade
                            let leverage = trade.leverage.unwrap_or(1) as f64;
                            let pnl = match trade.direction {
                                Direction::Long => (current_price - trade.entry_price) / trade.entry_price * trade.size_usd * leverage,
                                Direction::Short => (trade.entry_price - current_price) / trade.entry_price * trade.size_usd * leverage,
                            };
                            risk.record_trade_pnl(pnl);
                            {
                                let pnl_pct = (risk.current_equity() - initial_usd) / initial_usd * 100.0;
                                aggregator.update_daily_pnl(pnl_pct);
                            }

                            let record = TradeRecord {
                                id: trade.id,
                                ts_open: trade.opened_at,
                                ts_close: Some(chrono::Utc::now()),
                                duration_secs: Some((chrono::Utc::now() - trade.opened_at).num_seconds() as u64),
                                play_type: trade.play_type,
                                direction: trade.direction,
                                signal_level: trade.signal_level,
                                signal_score: trade.signal_score,
                                entry_price: trade.entry_price,
                                exit_price: Some(current_price),
                                size_usd: trade.size_usd,
                                leverage: trade.leverage,
                                pnl_usd: Some(pnl),
                                exit_reason: Some("expired".to_string()),
                                llm_reasoning: trade.llm_reasoning.clone(),
                                capital_after: Some(risk.current_equity()),
                            };
                            let _ = journal.log_entry(&record);

                            let _ = tui_tx_clone.send(TuiUpdate::Log(format!(
                                "[{}] Position {} expired — force closed, PnL: ${:.2}",
                                chrono::Utc::now().format("%H:%M:%S"),
                                trade.id, pnl,
                            ))).await;
                        }
                    }
                    risk.set_open_positions(position_monitor.open_count());

                    // Check SecondLook due entries
                    let due_entries = second_look.poll_due();
                    for entry in due_entries {
                        info!(bias = %entry.original_bias, attempt = entry.attempt, "SecondLook re-evaluation");
                        let sl_snapshot = aggregator.evaluate(&cfg);
                        match llm_gate.evaluate(&sl_snapshot, &cfg).await {
                            Ok(decision) => {
                                handle_llm_decision(
                                    &decision, &sl_snapshot, &cfg,
                                    &mut risk, &mut position_monitor, &mut second_look,
                                    &mut tuner, &hl_executor, &journal, &tui_tx_clone,
                                ).await;
                            }
                            Err(e) => warn!("SecondLook LLM error: {e:#}"),
                        }
                    }

                    // Main signal evaluation → LLM gate
                    if snapshot.level != SignalLevel::NoTrade && snapshot.level != SignalLevel::Weak {
                        tuner.record_signal_pass();

                        if let Err(reason) = risk.can_trade(&cfg) {
                            let _ = tui_tx_clone.send(TuiUpdate::Log(format!(
                                "[{}] Risk block: {}",
                                chrono::Utc::now().format("%H:%M:%S"),
                                reason,
                            ))).await;
                        } else {
                            let _ = tui_tx_clone.send(TuiUpdate::Log(format!(
                                "[{}] {} (net={}) → LLM gate...",
                                chrono::Utc::now().format("%H:%M:%S"),
                                snapshot.level,
                                snapshot.net_score,
                            ))).await;

                            match llm_gate.evaluate(&snapshot, &cfg).await {
                                Ok(decision) => {
                                    handle_llm_decision(
                                        &decision, &snapshot, &cfg,
                                        &mut risk, &mut position_monitor, &mut second_look,
                                        &mut tuner, &hl_executor, &journal, &tui_tx_clone,
                                    ).await;
                                }
                                Err(e) => {
                                    warn!("LLM gate error: {e:#}");
                                    let _ = tui_tx_clone.send(TuiUpdate::Log(format!(
                                        "[{}] LLM error: {e}",
                                        chrono::Utc::now().format("%H:%M:%S"),
                                    ))).await;
                                }
                            }
                        }
                    }

                    // Check tuning triggers
                    if let Some(trigger) = tuner.should_tune() {
                        info!(trigger = ?trigger, "Tuning triggered");
                        let recent_trades = journal.read_recent(20).unwrap_or_default();
                        let trades_summary: String = recent_trades.iter().map(|t| {
                            format!("#{} {} {} pnl=${:.2} level={} score={}",
                                t.id, t.direction, t.play_type,
                                t.pnl_usd.unwrap_or(0.0), t.signal_level, t.signal_score)
                        }).collect::<Vec<_>>().join("\n");

                        let tune_context = format!(
                            "Recent trades ({}):\n{}\n\nCurrent signal params:\n{}\n\nTrigger: {:?}",
                            recent_trades.len(),
                            if trades_summary.is_empty() { "No trades yet".to_string() } else { trades_summary },
                            serde_json::to_string_pretty(&cfg.signals).unwrap_or_default(),
                            trigger,
                        );

                        // Generate 20-trade report on trade-count triggers
                        let is_trade_count = matches!(&trigger, TuneTrigger::TradeCount(_));

                        drop(cfg); // Release read lock before write

                        match tune_client.tune(TUNE_SYSTEM_PROMPT, &tune_context).await {
                            Ok(tune_decision) => {
                                let mut cfg_write = config_for_engine.write().await;
                                tuner.apply_tune(&mut cfg_write, &tune_decision);
                                let _ = tui_tx_clone.send(TuiUpdate::Log(format!(
                                    "[{}] Signal engine tuned: {}",
                                    chrono::Utc::now().format("%H:%M:%S"),
                                    tune_decision.reasoning,
                                ))).await;
                            }
                            Err(e) => {
                                warn!("Tuning failed: {e:#}");
                                tuner.mark_tuned(); // Reset timer to avoid retry spam
                            }
                        }

                        // Report generation (only on trade-count triggers)
                        if is_trade_count {
                            let recent = journal.read_recent(20).unwrap_or_default();
                            if !recent.is_empty() {
                                match reporter.generate(&recent, &memory).await {
                                    Ok(result) => {
                                        let _ = tui_tx_clone.send(TuiUpdate::Log(format!(
                                            "[{}] Report generated: {}",
                                            chrono::Utc::now().format("%H:%M:%S"),
                                            result.summary,
                                        ))).await;
                                        // Telegram report alert
                                        if let Some(ref tg) = telegram {
                                            let s = result.summary.clone();
                                            let tg_ref = tg.clone();
                                            tokio::spawn(async move { tg_ref.send_report(&s).await });
                                        }
                                    }
                                    Err(e) => warn!("Report generation failed: {e:#}"),
                                }
                            }
                        }
                    } else {
                        drop(cfg);
                    }

                    // Auto kill switch check
                    {
                        let cfg = config_for_engine.read().await;
                        if risk.should_kill(&cfg) && !kill_switch.triggered {
                            kill_switch.trigger();
                            aggregator.set_kill_switch(true);
                            if let Some(ref hl) = hl_executor {
                                let _ = hl.close_all().await;
                            }
                            let _ = pm_executor.cancel_all().await;
                            let _ = tui_tx_clone.send(TuiUpdate::Log(format!(
                                "[{}] KILL SWITCH — drawdown limit hit",
                                chrono::Utc::now().format("%H:%M:%S"),
                            ))).await;
                            // Telegram kill switch alert
                            if let Some(ref tg) = telegram {
                                let tg_ref = tg.clone();
                                tokio::spawn(async move { tg_ref.send_kill_switch().await });
                            }
                        }
                    }
                }
                else => break,
            }
        }
    });

    // TUI on main thread
    let mut app = App::new(config.capital.initial_usd);

    crossterm::terminal::enable_raw_mode()?;
    std::io::stdout().execute(crossterm::terminal::EnterAlternateScreen)?;
    let mut terminal =
        ratatui::Terminal::new(ratatui::prelude::CrosstermBackend::new(std::io::stdout()))?;

    info!("All components initialized — entering main loop");

    loop {
        while let Ok(update) = tui_rx.try_recv() {
            match update {
                TuiUpdate::Price(p) => app.update_price(p),
                TuiUpdate::Signal(s) => app.update_signal(s),
                TuiUpdate::Log(l) => app.push_log(l),
                TuiUpdate::Positions(p) => app.update_positions(p),
                TuiUpdate::Stats(s) => app.update_stats(s),
            }
        }

        terminal.draw(|frame| app.render_frame(frame))?;

        if crossterm::event::poll(std::time::Duration::from_millis(50))? {
            if let crossterm::event::Event::Key(key) = crossterm::event::read()? {
                if key.kind == crossterm::event::KeyEventKind::Press {
                    match key.code {
                        crossterm::event::KeyCode::Char('q') => break,
                        crossterm::event::KeyCode::Char('k') => {
                            error!("KILL SWITCH activated by user");
                            break;
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    crossterm::terminal::disable_raw_mode()?;
    std::io::stdout().execute(crossterm::terminal::LeaveAlternateScreen)?;
    info!("openSigma v1 shutting down");
    Ok(())
}

/// Handle an LLM decision: execute trade, skip, or schedule second look.
async fn handle_llm_decision(
    decision: &LlmDecision,
    snapshot: &SignalSnapshot,
    config: &Config,
    risk: &mut RiskChecker,
    position_monitor: &mut PositionMonitor,
    second_look: &mut SecondLookScheduler,
    _tuner: &mut SignalTuner,
    hl_executor: &Option<HlExecutor>,
    journal: &TradeLogger,
    tui_tx: &mpsc::Sender<TuiUpdate>,
) {
    match decision {
        LlmDecision::Execute {
            play_type,
            direction,
            size_pct,
            hl_leverage,
            stop_loss_pct,
            take_profit_pct,
            pm_hedge,
            reasoning,
        } => {
            // Validate against hard risk limits
            if let Err(reason) = risk.validate_decision(decision, config) {
                warn!(reason = %reason, "LLM decision rejected by risk checker");
                let _ = tui_tx.send(TuiUpdate::Log(format!(
                    "[{}] Risk rejected: {}",
                    chrono::Utc::now().format("%H:%M:%S"),
                    reason,
                ))).await;
                return;
            }

            let size_usd = risk.max_trade_usd(config) * (size_pct / config.capital.max_trade_pct);
            let leverage = hl_leverage.unwrap_or(1);

            info!(
                play_type = %play_type, direction = %direction,
                size_usd = size_usd, leverage = leverage,
                "Executing trade"
            );

            // Place order via HL executor
            let mut entry_price = 0.0;
            if let Some(ref hl) = hl_executor {
                let is_buy = *direction == Direction::Long;
                let sz_coin = size_usd / snapshot.indicators.ema_9.unwrap_or(80000.0); // approximate
                match hl.market_order("BTC", is_buy, sz_coin, leverage).await {
                    Ok(result) => {
                        if result.success {
                            entry_price = result.filled_price.unwrap_or(0.0);
                            info!(price = entry_price, "Order filled");

                            // Place stop-loss and take-profit
                            let sl_price = if is_buy {
                                entry_price * (1.0 - stop_loss_pct / 100.0)
                            } else {
                                entry_price * (1.0 + stop_loss_pct / 100.0)
                            };
                            let tp_price = if is_buy {
                                entry_price * (1.0 + take_profit_pct / 100.0)
                            } else {
                                entry_price * (1.0 - take_profit_pct / 100.0)
                            };
                            let _ = hl.stop_loss("BTC", sl_price, sz_coin, !is_buy).await;
                            let _ = hl.take_profit("BTC", tp_price, sz_coin, !is_buy).await;
                        } else {
                            warn!(msg = %result.message, "Order failed");
                            let _ = tui_tx.send(TuiUpdate::Log(format!(
                                "[{}] Order failed: {}",
                                chrono::Utc::now().format("%H:%M:%S"),
                                result.message,
                            ))).await;
                            return;
                        }
                    }
                    Err(e) => {
                        error!("Order execution error: {e:#}");
                        return;
                    }
                }
            }

            // Track position
            let trade = ActiveTrade {
                id: Uuid::new_v4(),
                symbol: Symbol::BTC,
                direction: *direction,
                play_type: *play_type,
                entry_price,
                size_usd,
                leverage: Some(leverage),
                stop_loss_pct: *stop_loss_pct,
                take_profit_pct: *take_profit_pct,
                opened_at: chrono::Utc::now(),
                max_hold_secs: config.execution.max_trade_duration_secs,
                pm_hedge: pm_hedge.clone(),
                llm_reasoning: reasoning.clone(),
                signal_level: snapshot.level,
                signal_score: snapshot.net_score,
            };
            let trade_id = trade.id;
            let trade_opened_at = trade.opened_at;
            position_monitor.add_trade(trade);
            risk.set_open_positions(position_monitor.open_count());

            // Journal the open trade
            let open_record = TradeRecord {
                id: trade_id,
                ts_open: trade_opened_at,
                ts_close: None,
                duration_secs: None,
                play_type: *play_type,
                direction: *direction,
                signal_level: snapshot.level,
                signal_score: snapshot.net_score,
                entry_price,
                exit_price: None,
                size_usd,
                leverage: Some(leverage),
                pnl_usd: None,
                exit_reason: None,
                llm_reasoning: reasoning.clone(),
                capital_after: None,
            };
            let _ = journal.log_entry(&open_record);

            let _ = tui_tx.send(TuiUpdate::Log(format!(
                "[{}] EXECUTE {} {} ${:.0} @{:.0} lev={} — {}",
                chrono::Utc::now().format("%H:%M:%S"),
                play_type, direction, size_usd, entry_price, leverage,
                reasoning,
            ))).await;
        }

        LlmDecision::Skip { reasoning } => {
            info!(reasoning = %reasoning, "LLM decided to SKIP");
            let _ = tui_tx.send(TuiUpdate::Log(format!(
                "[{}] SKIP — {}",
                chrono::Utc::now().format("%H:%M:%S"),
                reasoning,
            ))).await;
        }

        LlmDecision::SecondLook {
            recheck_after_secs,
            what_to_watch,
            reasoning,
            ..
        } => {
            if second_look.schedule(decision) {
                info!(
                    recheck = recheck_after_secs,
                    watch = %what_to_watch,
                    "SecondLook scheduled"
                );
                let _ = tui_tx.send(TuiUpdate::Log(format!(
                    "[{}] SECOND_LOOK in {}s — {}",
                    chrono::Utc::now().format("%H:%M:%S"),
                    recheck_after_secs, reasoning,
                ))).await;
            } else {
                info!("Max SecondLooks reached, forcing SKIP");
                let _ = tui_tx.send(TuiUpdate::Log(format!(
                    "[{}] Max SecondLooks reached — forced SKIP",
                    chrono::Utc::now().format("%H:%M:%S"),
                ))).await;
            }
        }
    }
}

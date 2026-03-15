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
use chrono::Datelike;
use crossterm::ExecutableCommand;
use tokio::sync::{mpsc, RwLock};
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::agent::llm_client::LlmClient;
use crate::agent::llm_gate::LlmGate;
use crate::agent::second_look::SecondLookScheduler;
use crate::agent::tuner::{apply_report_adjustments, SignalTuner};
use crate::config::{Config, Secrets};
use crate::data::hyperliquid::{self, HyperliquidFeed};
use crate::data::news::NewsFeed;
use crate::execution::hyperliquid::{HlExecutor, HlPosition, query_account_state};
use ethers::signers::Signer as _;
use crate::execution::kill_switch::KillSwitch;
use crate::execution::position_monitor::{PositionEvent, PositionMonitor};
use crate::execution::risk::RiskChecker;
use crate::journal::logger::TradeLogger;
use crate::journal::memory::MemoryManager;
use crate::journal::reporter::Reporter;
use crate::signals::aggregator::SignalAggregator;
use crate::signals::indicators::Indicators;
use crate::telegram::TelegramClient;
use crate::tui::app::{App, ExchangeBalances, PerformanceStats, PositionInfo};
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

#[allow(clippy::large_enum_variant)]
enum TuiUpdate {
    Price(f64),
    Signal(SignalSnapshot),
    Log(String),
    Positions(Vec<PositionInfo>),
    Stats(PerformanceStats),
    Balances(ExchangeBalances),
}

/// Options for recording a trade close (tuner, telegram, stats vary by close reason).
struct CloseRecordOptions {
    record_tuner: bool,
    send_telegram: bool,
    send_stats: bool,
}

/// Record a closed trade: PnL, journal, aggregator, optional tuner/telegram/stats.
#[allow(clippy::too_many_arguments)]
async fn record_trade_close(
    trade: &ActiveTrade,
    exit_price: f64,
    exit_reason: &str,
    log_msg: &str,
    risk: &mut RiskChecker,
    initial_usd: f64,
    aggregator: &mut SignalAggregator,
    journal: &TradeLogger,
    tui_tx: &mpsc::Sender<TuiUpdate>,
    options: CloseRecordOptions,
    tuner: Option<&mut SignalTuner>,
    telegram: Option<&TelegramClient>,
) {
    let pnl = trade.compute_pnl(exit_price);
    risk.record_trade_pnl(pnl);
    let pnl_pct = if initial_usd > 0.0 {
        (risk.current_equity() - initial_usd) / initial_usd * 100.0
    } else {
        0.0
    };
    aggregator.update_daily_pnl(pnl_pct);

    if options.record_tuner {
        if let Some(t) = tuner {
            t.record_trade();
        }
    }

    let record = trade.to_closed_record(exit_price, exit_reason, pnl, risk.current_equity());
    let _ = journal.log_entry(&record);

    if options.send_telegram {
        if let Some(tg) = telegram {
            let tg_owned = tg.clone();
            let r = record.clone();
            tokio::spawn(async move { tg_owned.send_trade_close(&r).await });
        }
    }

    let _ = tui_tx.send(TuiUpdate::Log(log_msg.to_string())).await;

    if options.send_stats {
        let _ = tui_tx
            .send(TuiUpdate::Stats(PerformanceStats {
                total_trades: risk.total_closed(),
                win_rate: risk.win_rate(),
                total_pnl: risk.current_equity() - initial_usd,
                streak: risk.streak(),
            }))
            .await;
    }
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
    let mut config = Config::load("config.toml")?;
    config.load_tuned_signals("data/tuned_signals.toml");
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

    let mut news_feed = NewsFeed::new();
    tokio::spawn(async move { news_feed.run().await });

    // Initialize signal aggregator (BTC only)
    let mut aggregator = SignalAggregator::new();

    // Pre-load historical candles for BTC
    info!("Loading historical candles from Hyperliquid...");
    match hyperliquid::fetch_historical_candles("BTC", "1m", 200).await {
        Ok(candles) => {
            let count = candles.len();
            for c in candles {
                aggregator.indicators.push_candle_1m(c);
            }
            info!(count, "1m candles loaded");
        }
        Err(e) => warn!("Failed to load 1m candles: {e:#}"),
    }
    match hyperliquid::fetch_historical_candles("BTC", "5m", 200).await {
        Ok(candles) => {
            let count = candles.len();
            aggregator.indicators.seed_cvd_from_candles(&candles);
            for c in candles {
                aggregator.indicators.push_candle_5m(c);
            }
            info!(count, "5m candles loaded");
        }
        Err(e) => warn!("Failed to load 5m candles: {e:#}"),
    }

    let memory = Arc::new(MemoryManager::new("memory/memory.md"));
    let journal = TradeLogger::new("data/journal.jsonl");

    let llm_client = LlmClient::new(
        secrets.anthropic_api_key.clone(),
        config.llm.model.clone(),
        config.llm.timeout_ms,
    )?;
    let llm_gate = LlmGate::new(llm_client, memory.clone());

    // Separate LLM client for tuning (avoids borrow conflicts)
    let tune_client = LlmClient::new(
        secrets.anthropic_api_key.clone(),
        config.llm.model.clone(),
        config.llm.timeout_ms * 3, // longer timeout for tuning
    )?;

    // Reporter for 20-trade batch analysis
    let reporter = Reporter::new(&secrets.anthropic_api_key, &config.llm.model)?;

    // Telegram alerts (optional)
    let telegram = if config.telegram.enabled {
        TelegramClient::new(secrets.telegram_bot_token.clone(), secrets.telegram_chat_id.clone())
    } else {
        None
    };

    let mut second_look = SecondLookScheduler::new(config.execution.max_second_looks);
    let mut tuner = SignalTuner::new(&config);
    let mut position_monitor = PositionMonitor::new();
    let mut kill_switch = KillSwitch::new();

    let hl_executor = match HlExecutor::new(&secrets.hl_private_key).await {
        Ok(ex) => Some(ex),
        Err(e) => {
            warn!("HlExecutor init failed (trading disabled): {e:#}");
            None
        }
    };
    // Fetch real exchange balance at startup for accurate daily PnL tracking.
    // Falls back to config.capital.initial_usd if exchange queries fail.
    let startup_hl_equity = if let Some(ref hl) = hl_executor {
        hl.account_equity().await.unwrap_or(0.0)
    } else {
        0.0
    };
    let initial_usd = {
        let real = startup_hl_equity;
        if real > 0.0 {
            info!(real_balance = real, "Using real exchange balance as starting equity");
            real
        } else {
            warn!("Could not fetch exchange balance, falling back to config initial_usd");
            config.capital.initial_usd
        }
    };
    let mut risk = RiskChecker::new(initial_usd);

    // Restore stats from journal history
    match journal.read_all_closed() {
        Ok(trades) if !trades.is_empty() => {
            risk.restore_from_trades(&trades);
        }
        Err(e) => warn!("Failed to read journal for stats restoration: {e:#}"),
        _ => {}
    }

    // Startup reconciliation: detect positions left open from previous run.
    if let Some(ref hl) = hl_executor {
        match hl.positions().await {
            Ok(positions) => {
                let price_for_atr = positions.first().map(|p| p.entry_px).unwrap_or(71000.0);
                let recovered_sl = aggregator.indicators.atr_pct(price_for_atr)
                    .unwrap_or(0.2).min(0.3);
                let recovered_tp = (recovered_sl * 1.7).min(0.5);
                for pos in &positions {
                    let direction = if pos.size > 0.0 { Direction::Long } else { Direction::Short };
                    let size_usd = pos.size.abs() * pos.entry_px;
                    let trade = ActiveTrade {
                        id: Uuid::new_v4(),
                        symbol: Symbol::BTC,
                        direction,
                        play_type: PlayType::PurePerpScalp,
                        entry_price: pos.entry_px,
                        size_usd,
                        leverage: Some(pos.leverage),
                        stop_loss_pct: recovered_sl,
                        take_profit_pct: recovered_tp,
                        opened_at: chrono::Utc::now(),
                        max_hold_secs: config.execution.max_trade_duration_secs,
                        llm_reasoning: "Recovered from previous session".to_string(),
                        signal_level: SignalLevel::Weak,
                        signal_score: 0,
                        entry_rsi: None,
                        entry_cvd: None,
                        entry_ob: None,
                        entry_atr_pct: None,
                        entry_bb_position: None,
                    };
                    info!(
                        coin = %pos.coin, direction = %direction,
                        size = pos.size, entry = pos.entry_px,
                        "Recovered open position from HL"
                    );
                    position_monitor.add_trade(trade);
                }
                if !positions.is_empty() {
                    risk.set_open_positions(position_monitor.open_count(), &config);
                    info!(count = positions.len(), "Startup reconciliation complete");
                }
            }
            Err(e) => warn!("Failed to query HL positions on startup: {e:#}"),
        }
    }

    let eval_interval =
        tokio::time::Duration::from_secs(config.execution.signal_eval_interval_secs);
    let mut eval_ticker = tokio::time::interval(eval_interval);

    // TUI update channel
    let (tui_tx, mut tui_rx) = mpsc::channel::<TuiUpdate>(256);
    let tui_tx_clone = tui_tx.clone();

    // Send initial stats from journal history so TUI shows correct values on startup
    if risk.total_closed() > 0 {
        let _ = tui_tx.try_send(TuiUpdate::Stats(PerformanceStats {
            total_trades: risk.total_closed(),
            win_rate: risk.win_rate(),
            total_pnl: risk.current_equity() - initial_usd,
            streak: risk.streak(),
        }));
    }
    let config_for_engine = shared_config.clone();

    // Shared state — updated by poller, read by engine for risk + LLM context
    let shared_balance = Arc::new(RwLock::new(0.0f64));
    let balance_for_engine = shared_balance.clone();
    let shared_hl_positions: Arc<RwLock<Vec<HlPosition>>> = Arc::new(RwLock::new(Vec::new()));
    let hl_positions_for_engine = shared_hl_positions.clone();

    // Spawn account state poller (queries HL SDK every 15s for balances + positions)
    {
        let tui_tx_bal = tui_tx.clone();
        let hl_key = secrets.hl_private_key.clone();
        let bal_write = shared_balance.clone();
        let pos_write = shared_hl_positions.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(15));

            let wallet_address = match hl_key.parse::<ethers::signers::LocalWallet>() {
                Ok(w) => w.address(),
                Err(_) => {
                    warn!("Failed to parse HL key for balance poller");
                    return;
                }
            };

            loop {
                interval.tick().await;

                // Query HL via SDK: total equity, available USDC, and all positions
                match query_account_state(wallet_address).await {
                    Ok((hl_equity, hl_available, positions)) => {
                        let total = hl_equity;

                        // Update shared state for engine
                        *bal_write.write().await = total;
                        *pos_write.write().await = positions.clone();

                        // Convert HL positions to TUI format
                        let tui_positions: Vec<PositionInfo> = positions.iter().map(|p| {
                            let direction = if p.size > 0.0 { Direction::Long } else { Direction::Short };
                            PositionInfo {
                                coin: p.coin.clone(),
                                direction,
                                entry_price: p.entry_px,
                                notional: p.size.abs() * p.entry_px,
                                leverage: p.leverage,
                                unrealized_pnl: p.unrealized_pnl,
                            }
                        }).collect();

                        let _ = tui_tx_bal.send(TuiUpdate::Positions(tui_positions)).await;
                        let _ = tui_tx_bal.send(TuiUpdate::Balances(ExchangeBalances {
                            hl_equity,
                            hl_available,
                        })).await;
                    }
                    Err(e) => {
                        warn!("HL account state query failed: {e:#}");
                    }
                }
            }
        });
    }

    // Shutdown channel: 'k' = close positions and exit, 'q' = exit without closing
    #[derive(Clone, Copy)]
    enum ShutdownKind {
        Quit,  // Exit gracefully, leave positions open
        Kill,  // Close all positions then exit
    }
    let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<ShutdownKind>(1);
    let shutdown_tx_for_tui = shutdown_tx.clone();

    // Spawn the market event processor + signal engine + LLM gate
    let engine_handle = tokio::spawn(async move {
        const LLM_SKIP_COOLDOWN_SECS: i64 = 20;
        const LLM_EXECUTE_COOLDOWN_SECS: i64 = 45;
        let mut llm_cooldown_until: Option<chrono::DateTime<chrono::Utc>> = None;

        let mut latest_price: f64 = 0.0;

        loop {
            tokio::select! {
                Some(event) = market_rx.recv() => {
                    match &event {
                        MarketEvent::Price(tick) => {
                            if tick.symbol == Symbol::BTC {
                                aggregator.update_price(tick.price);
                                latest_price = tick.price;
                                let _ = tui_tx_clone.send(TuiUpdate::Price(tick.price)).await;
                            }

                            // Check position stop/TP levels
                            let events = position_monitor.check_price_levels(latest_price);
                            for pe in events {
                                let (id, reason) = match pe {
                                    PositionEvent::StopHit(id) => (id, "stop_loss"),
                                    PositionEvent::TakeProfitHit(id) => (id, "take_profit"),
                                };
                                if let Some(trade) = position_monitor.remove_trade(&id) {
                                    let exit_price = latest_price;
                                    if trade.entry_price <= 0.0 {
                                        warn!(id = %trade.id, "Skip close: invalid entry_price");
                                        position_monitor.add_trade(trade);
                                        continue;
                                    }
                                    if let Some(ref hl) = hl_executor {
                                        let is_buy = trade.direction == Direction::Short;
                                        let sz = trade.size_usd / trade.entry_price;
                                        let _ = hl.market_order(trade.symbol.hl_coin(), is_buy, sz, trade.leverage.unwrap_or(1)).await;
                                    }
                                    let pnl = trade.compute_pnl(exit_price);
                                    let log_msg = format!(
                                        "[{}] {} {} closed ({}) PnL: ${:.2}",
                                        chrono::Utc::now().format("%H:%M:%S"),
                                        trade.symbol, trade.direction, reason, pnl,
                                    );
                                    record_trade_close(
                                        &trade, exit_price, reason,
                                        &log_msg, &mut risk, initial_usd,
                                        &mut aggregator, &journal, &tui_tx_clone,
                                        CloseRecordOptions {
                                            record_tuner: true,
                                            send_telegram: true,
                                            send_stats: true,
                                        },
                                        Some(&mut tuner),
                                        telegram.as_ref(),
                                    ).await;
                                }
                            }
                            {
                                let c = config_for_engine.read().await;
                                risk.set_open_positions(position_monitor.open_count(), &c);
                            }
                        }
                        MarketEvent::Trade(trade) => {
                            if trade.symbol == Symbol::BTC {
                                let is_buy = trade.side == Direction::Long;
                                aggregator.indicators.add_trade(trade.size, is_buy);
                                aggregator.push_trade_for_candles(trade.price, trade.size, trade.timestamp);
                            }
                        }
                        MarketEvent::OrderBook(book) => {
                            if book.symbol == Symbol::BTC {
                                let imbalance = Indicators::ob_imbalance(&book.bids, &book.asks, 10);
                                aggregator.update_ob_imbalance(imbalance);
                            }
                        }
                        MarketEvent::Funding(tick) => {
                            if tick.symbol == Symbol::BTC {
                                aggregator.update_funding(tick.rate);
                            }
                        }
                        MarketEvent::Liquidation(_) => {}
                    }
                }
                _ = eval_ticker.tick() => {
                    let cfg = config_for_engine.read().await;

                    // Kill switch auto-reset MUST run before the triggered guard
                    {
                        let today = chrono::Utc::now().ordinal();
                        if cfg.capital.kill_switch_auto_reset
                            && kill_switch.triggered
                            && kill_switch.triggered_day().map(|d| d != today).unwrap_or(false)
                        {
                            kill_switch.reset();
                            aggregator.set_kill_switch(false);
                            let _ = tui_tx_clone.send(TuiUpdate::Log(format!(
                                "[{}] Kill switch reset (new UTC day) — trading resumed",
                                chrono::Utc::now().format("%H:%M:%S"),
                            ))).await;
                        }
                        if cfg.capital.kill_switch_enabled
                            && risk.should_kill(&cfg)
                            && !kill_switch.triggered
                        {
                            kill_switch.trigger();
                            aggregator.set_kill_switch(true);
                            if let Some(ref hl) = hl_executor {
                                let _ = hl.close_all().await;
                            }
                            let _ = tui_tx_clone.send(TuiUpdate::Log(format!(
                                "[{}] KILL SWITCH — drawdown limit hit",
                                chrono::Utc::now().format("%H:%M:%S"),
                            ))).await;
                            if let Some(ref tg) = telegram {
                                let tg_ref = tg.clone();
                                tokio::spawn(async move { tg_ref.send_kill_switch().await });
                            }
                        }
                    }

                    if kill_switch.triggered {
                        drop(cfg);
                        continue;
                    }

                    // Evaluate BTC signal
                    let snapshot = aggregator.evaluate(&cfg);
                    let _ = tui_tx_clone.send(TuiUpdate::Signal(snapshot.clone())).await;
                    let has_signal = snapshot.level != SignalLevel::NoTrade && snapshot.level != SignalLevel::Weak;

                    // Check position expirations (max hold time)
                    let expired = position_monitor.check_expirations();
                    for id in expired {
                        if let Some(trade) = position_monitor.remove_trade(&id) {
                            if trade.entry_price <= 0.0 {
                                warn!(id = %trade.id, "Skip expired close: invalid entry_price");
                                position_monitor.add_trade(trade);
                            } else {
                                info!(id = %trade.id, symbol = %trade.symbol, "Position expired (max hold time)");
                                let current_price = latest_price;
                                if let Some(ref hl) = hl_executor {
                                    let is_buy = trade.direction == Direction::Short;
                                    let sz = trade.size_usd / trade.entry_price;
                                    let _ = hl.market_order(trade.symbol.hl_coin(), is_buy, sz, trade.leverage.unwrap_or(1)).await;
                                }
                                let pnl = trade.compute_pnl(current_price);
                                let log_msg = format!(
                                    "[{}] {} position expired — force closed, PnL: ${:.2}",
                                    chrono::Utc::now().format("%H:%M:%S"),
                                    trade.symbol, pnl,
                                );
                                record_trade_close(
                                    &trade, current_price, "expired",
                                    &log_msg, &mut risk, initial_usd,
                                    &mut aggregator, &journal, &tui_tx_clone,
                                    CloseRecordOptions {
                                        record_tuner: false,
                                        send_telegram: false,
                                        send_stats: true,
                                    },
                                    None,
                                    telegram.as_ref(),
                                ).await;
                            }
                        }
                    }
                    risk.set_open_positions(position_monitor.open_count(), &cfg);

                    // Check SecondLook due entries
                    let due_entries = second_look.poll_due();
                    for entry in due_entries {
                        info!(bias = %entry.original_bias, attempt = entry.attempt, "SecondLook re-evaluation");
                        let sl_snapshot = aggregator.evaluate(&cfg);
                        let hl_pos = hl_positions_for_engine.read().await;
                        let sl_pos_ctx = build_position_context(&hl_pos, &risk, &cfg);
                                match llm_gate.evaluate(&sl_snapshot, &cfg, &sl_pos_ctx).await {
                                    Ok(decision) => {
                                        handle_llm_decision(
                                            &decision, &sl_snapshot, &cfg,
                                            &mut risk, &mut position_monitor, &mut second_look,
                                            &mut tuner, &hl_executor, &journal, &tui_tx_clone,
                                            &telegram, &mut aggregator, initial_usd, latest_price,
                                        ).await;
                            }
                            Err(e) => warn!("SecondLook LLM error: {e:#}"),
                        }
                    }

                    // Sync real balance + position count from exchange poller
                    {
                        let real_bal = *balance_for_engine.read().await;
                        if real_bal > 0.0 {
                            risk.sync_balance(real_bal);
                        }
                        let hl_pos_count = hl_positions_for_engine.read().await.len() as u32;
                        let pos_count = position_monitor.open_count().max(hl_pos_count);
                        risk.set_open_positions(pos_count, &cfg);
                        risk.maybe_reset_day();
                    }

                    // Main signal evaluation → LLM gate
                    if risk.is_at_max_positions() {
                        // At capacity — manage exits only
                    } else if has_signal {
                        tuner.record_signal_pass();

                        if let Err(reason) = risk.can_trade(&cfg) {
                            if !reason.starts_with("Cooldown") {
                                let _ = tui_tx_clone.send(TuiUpdate::Log(format!(
                                    "[{}] Risk block: {}",
                                    chrono::Utc::now().format("%H:%M:%S"),
                                    reason,
                                ))).await;
                            }
                        } else {
                            let now = chrono::Utc::now();
                            let cooldown_active = llm_cooldown_until
                                .map(|t| now < t)
                                .unwrap_or(false);

                            if !cooldown_active {
                                let _ = tui_tx_clone.send(TuiUpdate::Log(format!(
                                    "[{}] BTC {} (net={}) → LLM gate...",
                                    now.format("%H:%M:%S"),
                                    snapshot.level, snapshot.net_score,
                                ))).await;

                                let hl_pos = hl_positions_for_engine.read().await;
                                let pos_ctx = build_position_context(&hl_pos, &risk, &cfg);

                                match llm_gate.evaluate(&snapshot, &cfg, &pos_ctx).await {
                                    Ok(decision) => {
                                        let cd_secs = match &decision {
                                            LlmDecision::Skip { .. } => LLM_SKIP_COOLDOWN_SECS,
                                            LlmDecision::Execute { .. } => LLM_EXECUTE_COOLDOWN_SECS,
                                            LlmDecision::SecondLook { .. } => 0,
                                        };
                                        if cd_secs > 0 {
                                            llm_cooldown_until = Some(chrono::Utc::now() + chrono::Duration::seconds(cd_secs));
                                        }
                                        handle_llm_decision(
                                            &decision, &snapshot, &cfg,
                                            &mut risk, &mut position_monitor, &mut second_look,
                                            &mut tuner, &hl_executor, &journal, &tui_tx_clone,
                                            &telegram, &mut aggregator, initial_usd, latest_price,
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

                        drop(cfg); // Release read lock before write

                        match tune_client.tune(TUNE_SYSTEM_PROMPT, &tune_context).await {
                            Ok(tune_decision) => {
                                let mut cfg_write = config_for_engine.write().await;
                                tuner.apply_tune(&mut cfg_write, &tune_decision);
                                if let Err(e) = cfg_write.save_signals("data/tuned_signals.toml") {
                                    warn!("Failed to persist tuned signals: {e:#}");
                                }
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

                        // Report generation (updates memory.md) — run on all tune triggers
                        let recent = recent_trades; // Reuse from tune context
                        if !recent.is_empty() {
                            let result = {
                                let cfg_report = config_for_engine.read().await;
                                reporter.generate(&recent, &memory, &cfg_report).await
                            };
                            match result {
                                Ok(result) => {
                                    let _ = tui_tx_clone.send(TuiUpdate::Log(format!(
                                        "[{}] Report generated: {}",
                                        chrono::Utc::now().format("%H:%M:%S"),
                                        result.summary,
                                    ))).await;
                                    if !result.param_adjustments.is_empty() {
                                        let mut cfg = config_for_engine.write().await;
                                        apply_report_adjustments(&mut cfg, &result.param_adjustments);
                                        if let Err(e) = cfg.save_signals("data/tuned_signals.toml") {
                                            warn!("Failed to persist report-tuned signals: {e:#}");
                                        }
                                        let _ = tui_tx_clone.send(TuiUpdate::Log(format!(
                                            "[{}] Report applied {} param adjustments",
                                            chrono::Utc::now().format("%H:%M:%S"),
                                            result.param_adjustments.len(),
                                        ))).await;
                                    }
                                    if let Some(ref tg) = telegram {
                                        let s = result.summary.clone();
                                        let tg_ref = tg.clone();
                                        tokio::spawn(async move { tg_ref.send_report(&s).await });
                                    }
                                }
                                Err(e) => warn!("Report generation failed: {e:#}"),
                            };
                        }
                    } else {
                        drop(cfg);
                    }
                }
                Some(kind) = shutdown_rx.recv() => {
                    match kind {
                        ShutdownKind::Kill => {
                            info!("User kill switch — closing all HL positions before exit");
                            if let Some(ref hl) = hl_executor {
                                if let Err(e) = hl.close_all().await {
                                    error!("Failed to close positions on user kill: {e:#}");
                                }
                            }
                            let _ = tui_tx_clone.send(TuiUpdate::Log(format!(
                                "[{}] User kill — positions closed, shutting down",
                                chrono::Utc::now().format("%H:%M:%S"),
                            ))).await;
                        }
                        ShutdownKind::Quit => {
                            info!("User quit — shutting down (positions left open)");
                            let _ = tui_tx_clone.send(TuiUpdate::Log(format!(
                                "[{}] Quit — shutting down",
                                chrono::Utc::now().format("%H:%M:%S"),
                            ))).await;
                        }
                    }
                    break;
                }
                else => break,
            }
        }
    });

    // TUI on main thread
    let mut app = App::new(initial_usd, config.capital.initial_usd, config.capital.max_concurrent_positions);

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
                TuiUpdate::Balances(b) => {
                    app.update_balances(b);
                }
            }
        }

        terminal.draw(|frame| app.render_frame(frame))?;

        if crossterm::event::poll(std::time::Duration::from_millis(50))? {
            if let crossterm::event::Event::Key(key) = crossterm::event::read()? {
                if key.kind == crossterm::event::KeyEventKind::Press {
                    match key.code {
                        crossterm::event::KeyCode::Char('q') => {
                            let _ = shutdown_tx_for_tui.try_send(ShutdownKind::Quit);
                            break;
                        }
                        crossterm::event::KeyCode::Char('k') => {
                            error!("KILL SWITCH activated by user");
                            let _ = shutdown_tx_for_tui.try_send(ShutdownKind::Kill);
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

    // Wait for engine to finish (both 'q' and 'k' signal shutdown)
    if let Err(e) = engine_handle.await {
        warn!("Engine task join error: {e:#}");
    }
    info!("openSigma v1 shutting down");
    Ok(())
}

/// Handle an LLM decision: execute trade, skip, or schedule second look.
#[allow(clippy::too_many_arguments)]
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
    telegram: &Option<TelegramClient>,
    aggregator: &mut SignalAggregator,
    initial_usd: f64,
    latest_price: f64,
) {
    match decision {
        LlmDecision::Execute {
            play_type,
            direction,
            size_pct,
            hl_leverage,
            stop_loss_pct,
            take_profit_pct,
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

            // Close opposite-direction positions before opening new ones.
            // If we're going Long but have Shorts open, close the Shorts first.
            let opposite_ids = position_monitor.opposite_direction_ids(*direction);
            if !opposite_ids.is_empty() {
                let _ = tui_tx.send(TuiUpdate::Log(format!(
                    "[{}] Closing {} opposite-direction position(s) before reversing",
                    chrono::Utc::now().format("%H:%M:%S"),
                    opposite_ids.len(),
                ))).await;

                for opp_id in &opposite_ids {
                    if let Some(opp_trade) = position_monitor.remove_trade(opp_id) {
                        if opp_trade.entry_price <= 0.0 {
                            warn!(id = %opp_trade.id, "Skip reverse close: invalid entry_price");
                            position_monitor.add_trade(opp_trade);
                            continue;
                        }
                        if let Some(ref hl) = hl_executor {
                            let is_buy = opp_trade.direction == Direction::Short;
                            let sz = opp_trade.size_usd / opp_trade.entry_price;
                            let _ = hl.market_order("BTC", is_buy, sz, opp_trade.leverage.unwrap_or(1)).await;
                        }
                        let current_price = latest_price;
                        let pnl = opp_trade.compute_pnl(current_price);
                        let log_msg = format!(
                            "[{}] {} closed (reversed) PnL: ${:.2}",
                            chrono::Utc::now().format("%H:%M:%S"),
                            opp_trade.direction, pnl,
                        );
                        record_trade_close(
                            &opp_trade, current_price, "reversed",
                            &log_msg, risk, initial_usd,
                            aggregator, journal, tui_tx,
                            CloseRecordOptions {
                                record_tuner: false,
                                send_telegram: false,
                                send_stats: true,
                            },
                            None,
                            telegram.as_ref(),
                        ).await;
                    }
                }
                risk.set_open_positions(position_monitor.open_count(), config);
            }

            let (_in_session, size_mult) = config.active_session();
            let max_pct = config.capital.max_trade_pct.max(0.01);
            let mut size_usd = risk.max_trade_usd(config) * (size_pct / max_pct) * size_mult;
            // STRONG signals: allow more capital (strong_signal_size_mult)
            if matches!(snapshot.level, SignalLevel::StrongLong | SignalLevel::StrongShort) {
                size_usd *= config.capital.strong_signal_size_mult;
            }
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
                // size_usd is margin; notional = margin × leverage
                // HL requires minimum $10 notional per order
                let notional = (size_usd * leverage as f64).max(10.5);
                let price = if latest_price > 0.0 { latest_price } else { snapshot.indicators.ema_9.unwrap_or(80000.0) };
                let sz_coin = notional / price;
                match hl.market_order("BTC", is_buy, sz_coin, leverage).await {
                    Ok(result) => {
                        if result.success {
                            entry_price = result.filled_price.unwrap_or(price);
                            if entry_price <= 0.0 {
                                entry_price = price;
                            }
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

            // Fallback for paper mode or when filled_price was None
            let price_fallback = if latest_price > 0.0 { latest_price } else { snapshot.indicators.ema_9.unwrap_or(80000.0) };
            if entry_price <= 0.0 {
                entry_price = price_fallback;
            }

            // Track position (size_usd = notional, must match the actual HL order)
            let notional_usd = (size_usd * leverage as f64).max(10.5);
            let ind = &snapshot.indicators;
            let trade = ActiveTrade {
                id: Uuid::new_v4(),
                symbol: Symbol::BTC,
                direction: *direction,
                play_type: *play_type,
                entry_price,
                size_usd: notional_usd,
                leverage: Some(leverage),
                stop_loss_pct: *stop_loss_pct,
                take_profit_pct: *take_profit_pct,
                opened_at: chrono::Utc::now(),
                max_hold_secs: config.execution.max_trade_duration_secs,
                llm_reasoning: reasoning.clone(),
                signal_level: snapshot.level,
                signal_score: snapshot.net_score,
                entry_rsi: ind.rsi_14,
                entry_cvd: ind.cvd,
                entry_ob: ind.ob_imbalance,
                entry_atr_pct: ind.atr_pct,
                entry_bb_position: ind.bb_position,
            };
            let trade_id = trade.id;
            let trade_opened_at = trade.opened_at;
            position_monitor.add_trade(trade);
            risk.set_open_positions(position_monitor.open_count(), config);
            risk.record_trade_open();

            // Journal the open trade (size_usd = notional for consistency)
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
                size_usd: notional_usd,
                leverage: Some(leverage),
                pnl_usd: None,
                exit_reason: None,
                llm_reasoning: reasoning.clone(),
                capital_after: None,
                entry_rsi: ind.rsi_14,
                entry_cvd: ind.cvd,
                entry_ob: ind.ob_imbalance,
                entry_atr_pct: ind.atr_pct,
                entry_bb_position: ind.bb_position,
            };
            let _ = journal.log_entry(&open_record);

            // Telegram alert on trade open (notional for consistency)
            if let Some(ref tg) = telegram {
                let tg_ref = tg.clone();
                let pt = *play_type;
                let dir = *direction;
                let reason = reasoning.clone();
                tokio::spawn(async move {
                    tg_ref.send_trade_open(pt, dir, notional_usd, entry_price, leverage, &reason).await;
                });
            }

            let _ = tui_tx.send(TuiUpdate::Log(format!(
                "[{}] EXECUTE {} {} ${:.0} @{:.0} lev={} — {}",
                chrono::Utc::now().format("%H:%M:%S"),
                play_type, direction, notional_usd, entry_price, leverage,
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

/// Build portfolio + position context for LLM using real HL data.
fn build_position_context(
    hl_positions: &[HlPosition],
    risk: &RiskChecker,
    config: &Config,
) -> String {
    let equity = risk.current_equity();

    // Portfolio-level summary
    let mut lines = vec![format!(
        "PORTFOLIO: equity=${:.2} | daily_loss=${:.2} | max_daily_loss={:.1}% | session_trades={} | win_rate={:.0}% | streak={}",
        equity,
        risk.daily_loss_usd(),
        config.capital.max_daily_loss_pct,
        risk.total_closed(),
        risk.win_rate() * 100.0,
        risk.streak(),
    )];

    if hl_positions.is_empty() {
        lines.push("Open positions: NONE (0 exposure)".to_string());
        return lines.join("\n");
    }

    // Aggregate metrics from real HL positions
    let mut total_notional = 0.0;
    let mut total_margin = 0.0;
    let mut total_pnl = 0.0;
    let mut long_count = 0u32;
    let mut short_count = 0u32;
    let mut long_margin = 0.0;
    let mut short_margin = 0.0;

    let mut position_lines = Vec::new();
    for p in hl_positions {
        let notional = p.size.abs() * p.entry_px;
        let margin = if p.leverage > 0 { notional / p.leverage as f64 } else { notional };
        total_notional += notional;
        total_margin += margin;
        total_pnl += p.unrealized_pnl;
        let dir = if p.size > 0.0 { "Long" } else { "Short" };
        if p.size > 0.0 {
            long_count += 1;
            long_margin += margin;
        } else {
            short_count += 1;
            short_margin += margin;
        }
        position_lines.push(format!(
            "  {} {} notional=${:.0} margin=${:.2} @{:.2} lev={} PnL=${:+.2}",
            dir, p.coin, notional, margin, p.entry_px, p.leverage, p.unrealized_pnl,
        ));
    }

    // margin_heat = total margin / equity — represents actual capital at risk
    let margin_heat_pct = if equity > 0.0 { total_margin / equity * 100.0 } else { 0.0 };

    lines.push(format!(
        "EXPOSURE: {} positions ({}L/{}S) | margin_used=${:.2} | margin_heat={:.1}% of equity | notional=${:.0} | aggregate_PnL=${:+.2}",
        hl_positions.len(), long_count, short_count,
        total_margin, margin_heat_pct, total_notional, total_pnl,
    ));
    if long_margin > 0.0 {
        lines.push(format!("  Long: ${:.2} margin across {} position(s)", long_margin, long_count));
    }
    if short_margin > 0.0 {
        lines.push(format!("  Short: ${:.2} margin across {} position(s)", short_margin, short_count));
    }
    lines.push("Positions (from HL):".to_string());
    lines.extend(position_lines);
    lines.join("\n")
}

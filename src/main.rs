mod agent;
mod config;
mod data;
mod execution;
mod journal;
mod signals;
mod telegram;
mod tui;
mod types;

use std::collections::HashMap;
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
use crate::data::hyperliquid::{self, HyperliquidFeed};
use crate::data::news::NewsFeed;
use crate::execution::hyperliquid::{HlExecutor, HlPosition, query_account_state};
use ethers::signers::Signer as _;
use crate::execution::exit_manager::evaluate_proactive_exits;
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
{"adjustments":[{"param":"<name>","old_value":<current>,"new_value":<suggested>}],"reasoning":"...","strategy_pattern":"optional short pattern idea for current regime"}

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
- vwap_dev_reversion_pct (float, VWAP deviation threshold for anti-chasing/mean-reversion)
- vwap_weight (int, score weight for VWAP deviation signal)

Rules:
- Only suggest changes you are confident will improve performance
- Provide clear reasoning based on the trade data
- Keep adjustments small and conservative
- Use the recent price movement block (last minutes) to adapt to regime shifts.
- If you detect a regime shift, put a short actionable pattern in `strategy_pattern` (else empty string).
- If no changes needed, return empty adjustments array"#;

enum TuiUpdate {
    Price(f64),
    Signal(SignalSnapshot),
    Log(String),
    Positions(Vec<PositionInfo>),
    Stats(PerformanceStats),
    Balances(ExchangeBalances),
}

#[derive(Debug, Clone)]
struct MakerExitState {
    oid: u64,
    placed_at: chrono::DateTime<chrono::Utc>,
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
    let mut position_monitor = PositionMonitor::new();
    let mut kill_switch = KillSwitch::new();

    let hl_executor = match HlExecutor::new(&secrets.hl_private_key).await {
        Ok(ex) => Some(ex),
        Err(e) => {
            warn!("HlExecutor init failed (trading disabled): {e:#}");
            None
        }
    };
    // Fetch startup account state once so risk baseline and TUI baseline are consistent.
    // - `initial_usd` (risk): equity_for_risk (account_value when available)
    // - `initial_tui_equity` (display): total_balance from HL Balances tab (USDC total)
    let startup_state = match secrets.hl_private_key.parse::<ethers::signers::LocalWallet>() {
        Ok(w) => query_account_state(w.address()).await.ok(),
        Err(_) => None,
    };

    let initial_usd = startup_state
        .as_ref()
        .map(|(equity_for_risk, _, _, _)| *equity_for_risk)
        .filter(|v| *v > 0.0)
        .unwrap_or_else(|| {
            warn!("Could not fetch startup risk equity, falling back to config initial_usd");
            config.capital.initial_usd
        });
    info!(real_balance = initial_usd, "Using startup risk equity baseline");

    let initial_tui_equity = startup_state
        .as_ref()
        .map(|(_, total_balance, _, _)| *total_balance)
        .filter(|v| *v > 0.0)
        .unwrap_or(initial_usd);
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
    // Uses ATR-based TP/SL if available, otherwise defaults to 0.2%/0.3%.
    let recovered_sl = aggregator.indicators.atr_pct(71000.0)
        .unwrap_or(0.2).min(0.3);
    let recovered_tp = (recovered_sl * 1.7).min(0.5);
    if let Some(ref hl) = hl_executor {
        match hl.positions().await {
            Ok(positions) => {
                for pos in &positions {
                    let direction = if pos.size > 0.0 { Direction::Long } else { Direction::Short };
                    let size_usd = pos.size.abs() * pos.entry_px;
                    let trade = ActiveTrade {
                        id: Uuid::new_v4(),
                        symbol: Symbol::BTC,
                        direction,
                        play_type: PlayType::BTCPerpScalp,
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
                        entry_delta_divergence: None,
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

                // Query HL via SDK: equity for risk, total/available for display (HL Balances tab)
                match query_account_state(wallet_address).await {
                    Ok((equity_for_risk, total_balance, available_balance, positions)) => {
                        // Update shared state for engine (risk/sizing uses equity including unrealized PnL)
                        *bal_write.write().await = equity_for_risk;
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
                        // TUI: HL = Total Balance, Free = Available Balance (from HL Balances tab)
                        let _ = tui_tx_bal.send(TuiUpdate::Balances(ExchangeBalances {
                            hl_equity: total_balance,
                            hl_available: available_balance,
                        })).await;
                    }
                    Err(e) => {
                        warn!("HL account state query failed: {e:#}");
                    }
                }
            }
        });
    }

    // Shutdown channel: when user presses 'k', we signal engine to close positions and exit
    let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);
    let shutdown_tx_for_tui = shutdown_tx.clone();

    // Spawn the market event processor + signal engine + LLM gate
    let engine_handle = tokio::spawn(async move {
        const LLM_SKIP_COOLDOWN_SECS: i64 = 20;
        const LLM_EXECUTE_COOLDOWN_SECS: i64 = 45;
        const CLOSE_RETRY_BACKOFF_SECS: i64 = 3;
        let mut llm_cooldown_until: Option<chrono::DateTime<chrono::Utc>> = None;
        let mut last_signal_fingerprint: Option<String> = None;
        let mut last_signal_change_at = chrono::Utc::now();
        let mut last_idle_llm_at: Option<chrono::DateTime<chrono::Utc>> = None;

        let mut latest_price: f64 = 0.0;
        let mut close_retry_after: HashMap<Uuid, chrono::DateTime<chrono::Utc>> = HashMap::new();
        let mut peak_pnl_pct: HashMap<Uuid, f64> = HashMap::new();
        let mut maker_exit_orders: HashMap<Uuid, MakerExitState> = HashMap::new();

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
                                match pe {
                                    PositionEvent::StopHit(id) | PositionEvent::TakeProfitHit(id) => {
                                        // When a maker TP is resting, let exchange fill it passively.
                                        if matches!(&pe, PositionEvent::TakeProfitHit(_))
                                            && maker_exit_orders.contains_key(&id)
                                        {
                                            continue;
                                        }
                                        let reason = match pe {
                                            PositionEvent::StopHit(_) => "stop_loss",
                                            PositionEvent::TakeProfitHit(_) => "take_profit",
                                            _ => "unknown",
                                        };
                                        let now = chrono::Utc::now();
                                        if close_retry_after
                                            .get(&id)
                                            .is_some_and(|next_try| now < *next_try)
                                        {
                                            continue;
                                        }
                                        let mut close_fill_price: Option<f64> = None;
                                        if let Some(trade_snapshot) = position_monitor.get_trade(&id) {
                                            match close_trade_on_exchange(&trade_snapshot, &hl_executor).await {
                                                Ok(px) => close_fill_price = px,
                                                Err(e) => {
                                                    close_retry_after.insert(
                                                        id,
                                                        now + chrono::Duration::seconds(CLOSE_RETRY_BACKOFF_SECS),
                                                    );
                                                    let _ = tui_tx_clone.send(TuiUpdate::Log(format!(
                                                        "[{}] Close failed ({}) {} {}: {} — retry in {}s",
                                                        chrono::Utc::now().format("%H:%M:%S"),
                                                        reason,
                                                        trade_snapshot.symbol,
                                                        trade_snapshot.direction,
                                                        e,
                                                        CLOSE_RETRY_BACKOFF_SECS,
                                                    ))).await;
                                                    continue;
                                                }
                                            }
                                        }
                                        if let Some(trade) = position_monitor.remove_trade(&id) {
                                            clear_maker_exit_order(&id, &mut maker_exit_orders, &hl_executor).await;
                                            close_retry_after.remove(&id);
                                            peak_pnl_pct.remove(&id);
                                            let exit_price = close_fill_price.unwrap_or(latest_price);
                                            let fee_bps = {
                                                let c = config_for_engine.read().await;
                                                c.execution.fee_bps_per_side.max(0.0)
                                            };
                                            let (gross_pnl, fee_usd, pnl) =
                                                compute_trade_pnl_with_fees(&trade, exit_price, fee_bps);
                                            risk.record_trade_pnl(pnl);
                                            let pnl_pct = if initial_usd > 0.0 { (risk.current_equity() - initial_usd) / initial_usd * 100.0 } else { 0.0 };
                                            aggregator.update_daily_pnl(pnl_pct);
                                            tuner.record_trade();

                                            let record = TradeRecord {
                                                id: trade.id,
                                                ts_open: trade.opened_at,
                                                ts_close: Some(chrono::Utc::now()),
                                                duration_secs: Some(
                                                    (chrono::Utc::now() - trade.opened_at)
                                                        .num_seconds()
                                                        .max(0) as u64,
                                                ),
                                                play_type: trade.play_type,
                                                direction: trade.direction,
                                                signal_level: trade.signal_level,
                                                signal_score: trade.signal_score,
                                                entry_price: trade.entry_price,
                                                exit_price: Some(exit_price),
                                                size_usd: trade.size_usd,
                                                leverage: trade.leverage,
                                                pnl_usd: Some(pnl),
                                                exit_reason: Some(reason.to_string()),
                                                llm_reasoning: trade.llm_reasoning.clone(),
                                                capital_after: Some(risk.current_equity()),
                                                entry_rsi: trade.entry_rsi,
                                                entry_cvd: trade.entry_cvd,
                                                entry_ob: trade.entry_ob,
                                                entry_atr_pct: trade.entry_atr_pct,
                                                entry_bb_position: trade.entry_bb_position,
                                                entry_delta_divergence: trade.entry_delta_divergence.clone(),
                                            };
                                            let _ = journal.log_entry(&record);

                                            if let Some(ref tg) = telegram {
                                                let r = record.clone();
                                                let tg_ref = tg.clone();
                                                tokio::spawn(async move { tg_ref.send_trade_close(&r).await });
                                            }

                                            let _ = tui_tx_clone.send(TuiUpdate::Log(format!(
                                                "[{}] {} {} closed ({}) PnL: ${:.2} (gross ${:.2}, fee ${:.2})",
                                                chrono::Utc::now().format("%H:%M:%S"),
                                                trade.symbol, trade.direction, reason, pnl, gross_pnl, fee_usd,
                                            ))).await;

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
                        MarketEvent::OpenInterest(tick) => {
                            if tick.symbol == Symbol::BTC {
                                aggregator.update_open_interest(tick.value);
                            }
                        }
                        MarketEvent::Liquidation(_) => {}
                    }
                }
                _ = eval_ticker.tick() => {
                    if kill_switch.triggered {
                        continue;
                    }

                    let cfg = config_for_engine.read().await;

                    // Evaluate BTC signal
                    let snapshot = aggregator.evaluate(&cfg);
                    let _ = tui_tx_clone.send(TuiUpdate::Signal(snapshot.clone())).await;
                    let has_signal = snapshot.level != SignalLevel::NoTrade && snapshot.level != SignalLevel::Weak;
                    let signal_fingerprint = format!(
                        "{}|{}|{}",
                        snapshot.level,
                        snapshot.net_score,
                        snapshot.filter_reason.as_deref().unwrap_or("none"),
                    );
                    if last_signal_fingerprint
                        .as_ref()
                        .map(|s| s != &signal_fingerprint)
                        .unwrap_or(true)
                    {
                        last_signal_fingerprint = Some(signal_fingerprint);
                        last_signal_change_at = chrono::Utc::now();
                    }

                    // Check position expirations (max hold time)
                    let expired = position_monitor.check_expirations();
                    let mut any_expired_closed = false;
                    for id in expired {
                        let now = chrono::Utc::now();
                        if close_retry_after
                            .get(&id)
                            .is_some_and(|next_try| now < *next_try)
                        {
                            continue;
                        }
                        let mut close_fill_price: Option<f64> = None;
                        if let Some(trade_snapshot) = position_monitor.get_trade(&id) {
                            match close_trade_on_exchange(&trade_snapshot, &hl_executor).await {
                                Ok(px) => close_fill_price = px,
                                Err(e) => {
                                    close_retry_after.insert(
                                        id,
                                        now + chrono::Duration::seconds(CLOSE_RETRY_BACKOFF_SECS),
                                    );
                                    let _ = tui_tx_clone.send(TuiUpdate::Log(format!(
                                        "[{}] Expire close failed {} {}: {} — retry in {}s",
                                        chrono::Utc::now().format("%H:%M:%S"),
                                        trade_snapshot.symbol,
                                        trade_snapshot.direction,
                                        e,
                                        CLOSE_RETRY_BACKOFF_SECS,
                                    ))).await;
                                    continue;
                                }
                            }
                        }
                        if let Some(trade) = position_monitor.remove_trade(&id) {
                            clear_maker_exit_order(&id, &mut maker_exit_orders, &hl_executor).await;
                            close_retry_after.remove(&id);
                            peak_pnl_pct.remove(&id);
                            info!(id = %trade.id, symbol = %trade.symbol, "Position expired (max hold time)");
                            let current_price = close_fill_price.unwrap_or(latest_price);
                            let (gross_pnl, fee_usd, pnl) = compute_trade_pnl_with_fees(
                                &trade,
                                current_price,
                                cfg.execution.fee_bps_per_side.max(0.0),
                            );
                            risk.record_trade_pnl(pnl);
                            tuner.record_trade();
                            {
                                let pnl_pct = if initial_usd > 0.0 { (risk.current_equity() - initial_usd) / initial_usd * 100.0 } else { 0.0 };
                                aggregator.update_daily_pnl(pnl_pct);
                            }

                            let record = TradeRecord {
                                id: trade.id,
                                ts_open: trade.opened_at,
                                ts_close: Some(chrono::Utc::now()),
                                duration_secs: Some(
                                    (chrono::Utc::now() - trade.opened_at)
                                        .num_seconds()
                                        .max(0) as u64,
                                ),
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
                                entry_rsi: trade.entry_rsi,
                                entry_cvd: trade.entry_cvd,
                                entry_ob: trade.entry_ob,
                                entry_atr_pct: trade.entry_atr_pct,
                                entry_bb_position: trade.entry_bb_position,
                                entry_delta_divergence: trade.entry_delta_divergence.clone(),
                            };
                            let _ = journal.log_entry(&record);
                            if let Some(ref tg) = telegram {
                                let r = record.clone();
                                let tg_ref = tg.clone();
                                tokio::spawn(async move { tg_ref.send_trade_close(&r).await });
                            }

                            let _ = tui_tx_clone.send(TuiUpdate::Log(format!(
                                "[{}] {} position expired — force closed, PnL: ${:.2} (gross ${:.2}, fee ${:.2})",
                                chrono::Utc::now().format("%H:%M:%S"),
                                trade.symbol, pnl, gross_pnl, fee_usd,
                            ))).await;
                            any_expired_closed = true;
                        }
                    }
                    if any_expired_closed {
                        // Forced close should not leave us in a post-execute cooldown pause.
                        llm_cooldown_until = None;
                    }
                    // Proactive exits: harvest early edge and protect profits.
                    let proactive_exits = evaluate_proactive_exits(
                        position_monitor.active_trades(),
                        latest_price,
                        chrono::Utc::now(),
                        &cfg.execution,
                        &mut peak_pnl_pct,
                    );
                    for px in proactive_exits {
                        let id = px.id;
                        let reason = px.reason.clone();
                        let now = chrono::Utc::now();
                        if close_retry_after
                            .get(&id)
                            .is_some_and(|next_try| now < *next_try)
                        {
                            continue;
                        }
                        let mut close_fill_price: Option<f64> = None;
                        if let Some(trade_snapshot) = position_monitor.get_trade(&id) {
                            match close_trade_on_exchange(&trade_snapshot, &hl_executor).await {
                                Ok(px) => close_fill_price = px,
                                Err(e) => {
                                    close_retry_after.insert(
                                        id,
                                        now + chrono::Duration::seconds(CLOSE_RETRY_BACKOFF_SECS),
                                    );
                                    let _ = tui_tx_clone.send(TuiUpdate::Log(format!(
                                        "[{}] Proactive close failed ({}) {} {}: {} — retry in {}s [elapsed={}s net={:+.3}% peak={:+.3}%]",
                                        chrono::Utc::now().format("%H:%M:%S"),
                                        reason,
                                        trade_snapshot.symbol,
                                        trade_snapshot.direction,
                                        e,
                                        CLOSE_RETRY_BACKOFF_SECS,
                                        px.elapsed_secs,
                                        px.net_pnl_pct,
                                        px.peak_pnl_pct,
                                    ))).await;
                                    continue;
                                }
                            }
                        }
                        if let Some(trade) = position_monitor.remove_trade(&id) {
                            clear_maker_exit_order(&id, &mut maker_exit_orders, &hl_executor).await;
                            close_retry_after.remove(&id);
                            peak_pnl_pct.remove(&id);
                            let current_price = close_fill_price.unwrap_or(latest_price);
                            let (gross_pnl, fee_usd, pnl) = compute_trade_pnl_with_fees(
                                &trade,
                                current_price,
                                cfg.execution.fee_bps_per_side.max(0.0),
                            );
                            risk.record_trade_pnl(pnl);
                            tuner.record_trade();
                            let pnl_pct = if initial_usd > 0.0 {
                                (risk.current_equity() - initial_usd) / initial_usd * 100.0
                            } else {
                                0.0
                            };
                            aggregator.update_daily_pnl(pnl_pct);

                            let record = TradeRecord {
                                id: trade.id,
                                ts_open: trade.opened_at,
                                ts_close: Some(chrono::Utc::now()),
                                duration_secs: Some(
                                    (chrono::Utc::now() - trade.opened_at)
                                        .num_seconds()
                                        .max(0) as u64,
                                ),
                                play_type: trade.play_type,
                                direction: trade.direction,
                                signal_level: trade.signal_level,
                                signal_score: trade.signal_score,
                                entry_price: trade.entry_price,
                                exit_price: Some(current_price),
                                size_usd: trade.size_usd,
                                leverage: trade.leverage,
                                pnl_usd: Some(pnl),
                                exit_reason: Some(reason.clone()),
                                llm_reasoning: trade.llm_reasoning.clone(),
                                capital_after: Some(risk.current_equity()),
                                entry_rsi: trade.entry_rsi,
                                entry_cvd: trade.entry_cvd,
                                entry_ob: trade.entry_ob,
                                entry_atr_pct: trade.entry_atr_pct,
                                entry_bb_position: trade.entry_bb_position,
                                entry_delta_divergence: trade.entry_delta_divergence.clone(),
                            };
                            let _ = journal.log_entry(&record);
                            if let Some(ref tg) = telegram {
                                let r = record.clone();
                                let tg_ref = tg.clone();
                                tokio::spawn(async move { tg_ref.send_trade_close(&r).await });
                            }
                            let _ = tui_tx_clone.send(TuiUpdate::Log(format!(
                                "[{}] Proactive exit ({}) {} {} PnL: ${:.2} (gross ${:.2}, fee ${:.2}) [elapsed={}s net={:+.3}% peak={:+.3}%]",
                                chrono::Utc::now().format("%H:%M:%S"),
                                reason,
                                trade.symbol,
                                trade.direction,
                                pnl,
                                gross_pnl,
                                fee_usd,
                                px.elapsed_secs,
                                px.net_pnl_pct,
                                px.peak_pnl_pct,
                            ))).await;
                        }
                    }

                    // Maker exit watchdog: if passive TP does not fill in time, fall back to taker close.
                    if cfg.execution.enable_maker_exit {
                        let now = chrono::Utc::now();
                        let mut stale_maker_ids = Vec::new();
                        let mut maker_timeout_ids = Vec::new();
                        for (id, state) in &maker_exit_orders {
                            if position_monitor.get_trade(id).is_none() {
                                stale_maker_ids.push(*id);
                                continue;
                            }
                            let age_secs = (now - state.placed_at).num_seconds();
                            if age_secs >= cfg.execution.maker_exit_timeout_secs as i64 {
                                maker_timeout_ids.push(*id);
                            }
                        }

                        for id in stale_maker_ids {
                            clear_maker_exit_order(&id, &mut maker_exit_orders, &hl_executor).await;
                        }

                        for id in maker_timeout_ids {
                            let now = chrono::Utc::now();
                            if close_retry_after
                                .get(&id)
                                .is_some_and(|next_try| now < *next_try)
                            {
                                continue;
                            }
                            let mut close_fill_price: Option<f64> = None;
                            if let Some(trade_snapshot) = position_monitor.get_trade(&id) {
                                clear_maker_exit_order(&id, &mut maker_exit_orders, &hl_executor).await;
                                match close_trade_on_exchange(&trade_snapshot, &hl_executor).await {
                                    Ok(px) => close_fill_price = px,
                                    Err(e) => {
                                        close_retry_after.insert(
                                            id,
                                            now + chrono::Duration::seconds(CLOSE_RETRY_BACKOFF_SECS),
                                        );
                                        let _ = tui_tx_clone.send(TuiUpdate::Log(format!(
                                            "[{}] Maker timeout fallback failed {} {}: {} — retry in {}s",
                                            chrono::Utc::now().format("%H:%M:%S"),
                                            trade_snapshot.symbol,
                                            trade_snapshot.direction,
                                            e,
                                            CLOSE_RETRY_BACKOFF_SECS,
                                        ))).await;
                                        continue;
                                    }
                                }
                            }

                            if let Some(trade) = position_monitor.remove_trade(&id) {
                                close_retry_after.remove(&id);
                                peak_pnl_pct.remove(&id);
                                let current_price = close_fill_price.unwrap_or(latest_price);
                                let (gross_pnl, fee_usd, pnl) = compute_trade_pnl_with_fees(
                                    &trade,
                                    current_price,
                                    cfg.execution.fee_bps_per_side.max(0.0),
                                );
                                risk.record_trade_pnl(pnl);
                                tuner.record_trade();
                                let pnl_pct = if initial_usd > 0.0 {
                                    (risk.current_equity() - initial_usd) / initial_usd * 100.0
                                } else {
                                    0.0
                                };
                                aggregator.update_daily_pnl(pnl_pct);

                                let record = TradeRecord {
                                    id: trade.id,
                                    ts_open: trade.opened_at,
                                    ts_close: Some(chrono::Utc::now()),
                                    duration_secs: Some(
                                        (chrono::Utc::now() - trade.opened_at)
                                            .num_seconds()
                                            .max(0) as u64,
                                    ),
                                    play_type: trade.play_type,
                                    direction: trade.direction,
                                    signal_level: trade.signal_level,
                                    signal_score: trade.signal_score,
                                    entry_price: trade.entry_price,
                                    exit_price: Some(current_price),
                                    size_usd: trade.size_usd,
                                    leverage: trade.leverage,
                                    pnl_usd: Some(pnl),
                                    exit_reason: Some("maker_timeout".to_string()),
                                    llm_reasoning: trade.llm_reasoning.clone(),
                                    capital_after: Some(risk.current_equity()),
                                    entry_rsi: trade.entry_rsi,
                                    entry_cvd: trade.entry_cvd,
                                    entry_ob: trade.entry_ob,
                                    entry_atr_pct: trade.entry_atr_pct,
                                    entry_bb_position: trade.entry_bb_position,
                                    entry_delta_divergence: trade.entry_delta_divergence.clone(),
                                };
                                let _ = journal.log_entry(&record);
                                if let Some(ref tg) = telegram {
                                    let r = record.clone();
                                    let tg_ref = tg.clone();
                                    tokio::spawn(async move { tg_ref.send_trade_close(&r).await });
                                }
                                let _ = tui_tx_clone.send(TuiUpdate::Log(format!(
                                    "[{}] Maker timeout fallback close {} {} PnL: ${:.2} (gross ${:.2}, fee ${:.2})",
                                    chrono::Utc::now().format("%H:%M:%S"),
                                    trade.symbol,
                                    trade.direction,
                                    pnl,
                                    gross_pnl,
                                    fee_usd,
                                ))).await;
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
                                    &mut tuner, &mut peak_pnl_pct, &mut maker_exit_orders, &hl_executor, &journal, &tui_tx_clone,
                                    &telegram, latest_price,
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
                                            &mut tuner, &mut peak_pnl_pct, &mut maker_exit_orders, &hl_executor, &journal, &tui_tx_clone,
                                            &telegram, latest_price,
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
                    } else {
                        // If signal remains unchanged and weak/no-trade for too long, ping LLM once
                        // so logs/context keep moving while scan continues.
                        let now = chrono::Utc::now();
                        let idle_secs = (now - last_signal_change_at).num_seconds();
                        let ping_due = idle_secs >= cfg.execution.idle_llm_ping_secs as i64;
                        let ping_gap_ok = last_idle_llm_at
                            .map(|t| (now - t).num_seconds() >= cfg.execution.idle_llm_ping_secs as i64)
                            .unwrap_or(true);
                        let cooldown_active = llm_cooldown_until
                            .map(|t| now < t)
                            .unwrap_or(false);

                        if ping_due && ping_gap_ok && !cooldown_active {
                            if let Err(reason) = risk.can_trade(&cfg) {
                                if !reason.starts_with("Cooldown") {
                                    let _ = tui_tx_clone.send(TuiUpdate::Log(format!(
                                        "[{}] Idle LLM ping blocked by risk: {}",
                                        now.format("%H:%M:%S"),
                                        reason,
                                    ))).await;
                                }
                            } else {
                                let _ = tui_tx_clone.send(TuiUpdate::Log(format!(
                                    "[{}] Idle market ({idle_secs}s unchanged) → LLM heartbeat",
                                    now.format("%H:%M:%S"),
                                ))).await;
                                let hl_pos = hl_positions_for_engine.read().await;
                                let pos_ctx = build_position_context(&hl_pos, &risk, &cfg);
                                last_idle_llm_at = Some(now);
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
                                            &mut tuner, &mut peak_pnl_pct, &mut maker_exit_orders, &hl_executor, &journal, &tui_tx_clone,
                                            &telegram, latest_price,
                                        ).await;
                                    }
                                    Err(e) => {
                                        warn!("Idle LLM heartbeat error: {e:#}");
                                        let _ = tui_tx_clone.send(TuiUpdate::Log(format!(
                                            "[{}] Idle LLM error: {e}",
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
                        let tune_batch_n = cfg.tuning.tune_every_n_trades.max(1) as usize;
                        let recent_trades = journal.read_recent(tune_batch_n).unwrap_or_default();
                        let movement_summary = aggregator
                            .indicators
                            .recent_price_movement_summary(8);
                        let trades_summary: String = recent_trades.iter().map(|t| {
                            format!("#{} {} {} pnl=${:.2} level={} score={}",
                                t.id, t.direction, t.play_type,
                                t.pnl_usd.unwrap_or(0.0), t.signal_level, t.signal_score)
                        }).collect::<Vec<_>>().join("\n");

                        let tune_context = format!(
                            "Recent trades ({}):\n{}\n\n{}\n\nCurrent signal params:\n{}\n\nTrigger: {:?}",
                            recent_trades.len(),
                            if trades_summary.is_empty() { "No trades yet".to_string() } else { trades_summary },
                            movement_summary,
                            serde_json::to_string_pretty(&cfg.signals).unwrap_or_default(),
                            trigger,
                        );

                        // Generate report on trade-count triggers (same batch size as tuning cadence)
                        let is_trade_count = matches!(&trigger, TuneTrigger::TradeCount(_));

                        drop(cfg); // Release read lock before write

                        match tune_client.tune(TUNE_SYSTEM_PROMPT, &tune_context).await {
                            Ok(tune_decision) => {
                                let mut cfg_write = config_for_engine.write().await;
                                tuner.apply_tune(&mut cfg_write, &tune_decision);
                                if let Err(e) = cfg_write.save_signals("data/tuned_signals.toml") {
                                    warn!("Failed to persist tuned signals: {e:#}");
                                }
                                // Always record tuning takeaways to memory (not only trade-count reports).
                                let trigger_label = format!("{trigger:?}");
                                let adjustments = if tune_decision.adjustments.is_empty() {
                                    "- none".to_string()
                                } else {
                                    tune_decision
                                        .adjustments
                                        .iter()
                                        .map(|a| {
                                            format!(
                                                "- {}: {:.4} -> {:.4}",
                                                a.param, a.old_value, a.new_value
                                            )
                                        })
                                        .collect::<Vec<_>>()
                                        .join("\n")
                                };
                                let tuned_snapshot = format!(
                                    "strong_threshold={} lean_threshold={} min_atr_pct={} rsi_oversold={} rsi_overbought={} vwap_dev_reversion_pct={} vwap_weight={} cvd_weight={} ob_weight={}",
                                    cfg_write.signals.strong_threshold,
                                    cfg_write.signals.lean_threshold,
                                    cfg_write.signals.min_atr_pct,
                                    cfg_write.signals.rsi_oversold,
                                    cfg_write.signals.rsi_overbought,
                                    cfg_write.signals.vwap_dev_reversion_pct,
                                    cfg_write.signals.vwap_weight,
                                    cfg_write.signals.cvd_weight,
                                    cfg_write.signals.ob_weight,
                                );
                                let memory_block = format!(
                                    "\n## Tune — {}\n### Trigger\n{}\n\n### Adjustments\n{}\n\n### Why\n{}\n\n### Strategy Pattern\n{}\n\n### Parameter Snapshot\n{}",
                                    chrono::Utc::now().format("%Y-%m-%d %H:%M UTC"),
                                    trigger_label,
                                    adjustments,
                                    tune_decision.reasoning,
                                    if tune_decision.strategy_pattern.trim().is_empty() {
                                        "none".to_string()
                                    } else {
                                        tune_decision.strategy_pattern.clone()
                                    },
                                    tuned_snapshot,
                                );
                                if let Err(e) = memory.append(&memory_block) {
                                    warn!("Failed to append tune memory: {e:#}");
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

                        // Report generation (only on trade-count triggers)
                        if is_trade_count {
                            let recent = journal.read_recent(tune_batch_n).unwrap_or_default();
                            if !recent.is_empty() {
                                let params_str = {
                                    let c = config_for_engine.read().await;
                                    format!(
                                        "strong_threshold={} lean_threshold={} min_atr_pct={} rsi_oversold={} rsi_overbought={}",
                                        c.signals.strong_threshold,
                                        c.signals.lean_threshold,
                                        c.signals.min_atr_pct,
                                        c.signals.rsi_oversold,
                                        c.signals.rsi_overbought,
                                    )
                                };
                                match reporter.generate(&recent, &memory, &params_str).await {
                                    Ok(result) => {
                                        if !result.param_adjustments.is_empty() {
                                            let mut cfg_write = config_for_engine.write().await;
                                            crate::journal::reporter::apply_param_adjustments(&mut cfg_write, &result.param_adjustments);
                                            if let Err(e) = cfg_write.save_signals("data/tuned_signals.toml") {
                                                tracing::warn!("Failed to persist tuned signals after report: {e:#}");
                                            }
                                        }
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
                _ = shutdown_rx.recv() => {
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
                    break;
                }
                else => break,
            }
        }
    });

    // TUI on main thread
    let fee_round_trip_pct =
        (config.execution.fee_bps_per_side.max(0.0) * 2.0) / 100.0
        + config.execution.sell_trigger_extra_buffer_pct.max(0.0);
    let mut app = App::new(
        initial_tui_equity,
        config.capital.initial_usd,
        fee_round_trip_pct,
    );

    crossterm::terminal::enable_raw_mode()?;
    std::io::stdout().execute(crossterm::terminal::EnterAlternateScreen)?;
    let mut terminal =
        ratatui::Terminal::new(ratatui::prelude::CrosstermBackend::new(std::io::stdout()))?;

    info!("All components initialized — entering main loop");

    let mut user_kill = false;
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
                        crossterm::event::KeyCode::Char('q') => break,
                        crossterm::event::KeyCode::Char('k') => {
                            error!("KILL SWITCH activated by user");
                            let _ = shutdown_tx_for_tui.try_send(());
                            user_kill = true;
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

    // If user pressed 'k', engine was signaled to close positions — wait for it to finish
    if user_kill {
        if let Err(e) = engine_handle.await {
            warn!("Engine task join error: {e:#}");
        }
    }
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
    tuner: &mut SignalTuner,
    peak_pnl_pct: &mut HashMap<Uuid, f64>,
    maker_exit_orders: &mut HashMap<Uuid, MakerExitState>,
    hl_executor: &Option<HlExecutor>,
    journal: &TradeLogger,
    tui_tx: &mpsc::Sender<TuiUpdate>,
    telegram: &Option<TelegramClient>,
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
                    if let Some(opp_trade) = position_monitor.get_trade(opp_id) {
                        let close_fill_price = match close_trade_on_exchange(&opp_trade, hl_executor).await {
                            Ok(px) => px,
                            Err(e) => {
                                let _ = tui_tx.send(TuiUpdate::Log(format!(
                                    "[{}] Close failed before reverse ({}): {} — keep position",
                                    chrono::Utc::now().format("%H:%M:%S"),
                                    opp_trade.direction,
                                    e,
                                ))).await;
                                return;
                            }
                        };
                        let Some(opp_trade) = position_monitor.remove_trade(opp_id) else {
                            continue;
                        };
                        clear_maker_exit_order(opp_id, maker_exit_orders, hl_executor).await;
                        peak_pnl_pct.remove(opp_id);
                        let current_price = close_fill_price.unwrap_or(latest_price);
                        let (gross_pnl, fee_usd, pnl) = compute_trade_pnl_with_fees(
                            &opp_trade,
                            current_price,
                            config.execution.fee_bps_per_side.max(0.0),
                        );
                        risk.record_trade_pnl(pnl);
                        tuner.record_trade();
                        let record = TradeRecord {
                            id: opp_trade.id,
                            ts_open: opp_trade.opened_at,
                            ts_close: Some(chrono::Utc::now()),
                            duration_secs: Some(
                                (chrono::Utc::now() - opp_trade.opened_at)
                                    .num_seconds()
                                    .max(0) as u64,
                            ),
                            play_type: opp_trade.play_type,
                            direction: opp_trade.direction,
                            signal_level: opp_trade.signal_level,
                            signal_score: opp_trade.signal_score,
                            entry_price: opp_trade.entry_price,
                            exit_price: Some(current_price),
                            size_usd: opp_trade.size_usd,
                            leverage: opp_trade.leverage,
                            pnl_usd: Some(pnl),
                            exit_reason: Some("reversed".to_string()),
                            llm_reasoning: opp_trade.llm_reasoning.clone(),
                            capital_after: Some(risk.current_equity()),
                            entry_rsi: opp_trade.entry_rsi,
                            entry_cvd: opp_trade.entry_cvd,
                            entry_ob: opp_trade.entry_ob,
                            entry_atr_pct: opp_trade.entry_atr_pct,
                            entry_bb_position: opp_trade.entry_bb_position,
                            entry_delta_divergence: opp_trade.entry_delta_divergence.clone(),
                        };
                        let _ = journal.log_entry(&record);
                        if let Some(ref tg) = telegram {
                            let r = record.clone();
                            let tg_ref = tg.clone();
                            tokio::spawn(async move { tg_ref.send_trade_close(&r).await });
                        }
                        let _ = tui_tx.send(TuiUpdate::Log(format!(
                            "[{}] {} closed (reversed) PnL: ${:.2} (gross ${:.2}, fee ${:.2})",
                            chrono::Utc::now().format("%H:%M:%S"),
                            opp_trade.direction, pnl, gross_pnl, fee_usd,
                        ))).await;
                    }
                }
                risk.set_open_positions(position_monitor.open_count(), config);
            }

            let (_in_session, size_mult) = config.active_session();
            let mut effective_size_pct = *size_pct;
            if matches!(snapshot.level, SignalLevel::StrongLong | SignalLevel::StrongShort) {
                // Avoid under-betting on high-conviction signals.
                let strong_floor = (config.capital.max_trade_pct * 0.45).min(config.capital.max_trade_pct);
                if effective_size_pct < strong_floor {
                    effective_size_pct = strong_floor;
                }
            }
            let base_size = risk.max_trade_usd(config)
                * (effective_size_pct / config.capital.max_trade_pct)
                * size_mult;
            let size_usd = if matches!(snapshot.level, SignalLevel::StrongLong | SignalLevel::StrongShort) {
                base_size * config.capital.strong_signal_size_mult
            } else {
                base_size
            };
            let leverage = hl_leverage.unwrap_or(1);

            info!(
                play_type = %play_type, direction = %direction,
                llm_size_pct = *size_pct, effective_size_pct = effective_size_pct,
                size_usd = size_usd, leverage = leverage,
                "Executing trade"
            );

            let round_trip_fee_pct = (config.execution.fee_bps_per_side.max(0.0) * 2.0) / 100.0;
            let tp_fee_floor_pct = round_trip_fee_pct + config.execution.sell_trigger_extra_buffer_pct.max(0.0);
            let effective_take_profit_pct = take_profit_pct.max(tp_fee_floor_pct);
            let (prefer_maker_exit, exit_mode_reason) = choose_exit_mode(snapshot, config);
            let trade_id = Uuid::new_v4();

            // Place order via HL executor
            let mut entry_price = 0.0;
            let mut maker_exit_oid: Option<u64> = None;
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
                            entry_price = result.filled_price.unwrap_or(0.0);
                            info!(price = entry_price, "Order filled");

                            // Place stop-loss and take-profit
                            let sl_price = if is_buy {
                                entry_price * (1.0 - stop_loss_pct / 100.0)
                            } else {
                                entry_price * (1.0 + stop_loss_pct / 100.0)
                            };
                            let tp_price = if is_buy {
                                entry_price * (1.0 + effective_take_profit_pct / 100.0)
                            } else {
                                entry_price * (1.0 - effective_take_profit_pct / 100.0)
                            };
                            let _ = hl.stop_loss("BTC", sl_price, sz_coin, !is_buy).await;
                            if prefer_maker_exit {
                                let maker_tp_price = if is_buy {
                                    tp_price * (1.0 + config.execution.maker_exit_markup_pct.max(0.0) / 100.0)
                                } else {
                                    tp_price * (1.0 - config.execution.maker_exit_markup_pct.max(0.0) / 100.0)
                                };
                                match hl
                                    .reduce_only_limit("BTC", maker_tp_price, sz_coin, !is_buy, true)
                                    .await
                                {
                                    Ok(tp_res) if tp_res.success => {
                                        maker_exit_oid = tp_res
                                            .order_id
                                            .as_deref()
                                            .and_then(|s| s.parse::<u64>().ok());
                                        if maker_exit_oid.is_none() {
                                            let _ = hl.take_profit("BTC", tp_price, sz_coin, !is_buy).await;
                                        }
                                    }
                                    Ok(tp_res) => {
                                        warn!(msg = %tp_res.message, "Maker TP failed, fallback to trigger TP");
                                        let _ = hl.take_profit("BTC", tp_price, sz_coin, !is_buy).await;
                                    }
                                    Err(e) => {
                                        warn!("Maker TP placement error, fallback to trigger TP: {e:#}");
                                        let _ = hl.take_profit("BTC", tp_price, sz_coin, !is_buy).await;
                                    }
                                }
                            } else {
                                let _ = hl.take_profit("BTC", tp_price, sz_coin, !is_buy).await;
                            }
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

            // Track position (size_usd = notional, must match the actual HL order)
            let notional_usd = (size_usd * leverage as f64).max(10.5);
            let ind = &snapshot.indicators;
            let trade = ActiveTrade {
                id: trade_id,
                symbol: Symbol::BTC,
                direction: *direction,
                play_type: *play_type,
                entry_price,
                size_usd: notional_usd,
                leverage: Some(leverage),
                stop_loss_pct: *stop_loss_pct,
                take_profit_pct: effective_take_profit_pct,
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
                entry_delta_divergence: ind.delta_divergence.clone(),
            };
            let trade_opened_at = trade.opened_at;
            position_monitor.add_trade(trade);
            peak_pnl_pct.insert(trade_id, 0.0);
            if let Some(oid) = maker_exit_oid {
                maker_exit_orders.insert(
                    trade_id,
                    MakerExitState {
                        oid,
                        placed_at: chrono::Utc::now(),
                    },
                );
            }
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
                entry_delta_divergence: ind.delta_divergence.clone(),
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
                "[{}] EXECUTE {} {} ${:.0} @{:.0} lev={} tp={:.3}% (req {:.3}%) exit={} ({}) — {}",
                chrono::Utc::now().format("%H:%M:%S"),
                play_type, direction, notional_usd, entry_price, leverage,
                effective_take_profit_pct, take_profit_pct,
                if maker_exit_oid.is_some() { "maker" } else { "trigger" },
                exit_mode_reason,
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

/// Decide whether to use maker-first exit or trigger (taker-style) TP.
/// High-volatility / trend conditions favor faster taker-style exits.
fn choose_exit_mode(snapshot: &SignalSnapshot, config: &Config) -> (bool, String) {
    if !config.execution.enable_maker_exit {
        return (false, "maker_disabled".to_string());
    }

    let atr_pct = snapshot.indicators.atr_pct.unwrap_or(0.0);
    let regime = snapshot.indicators.regime.as_deref();
    let is_trend = matches!(regime, Some("trend_up") | Some("trend_down"));
    let max_atr = if is_trend {
        config.execution.maker_exit_trend_atr_pct
    } else {
        config.execution.maker_exit_max_atr_pct
    };

    if atr_pct > max_atr {
        return (
            false,
            format!("high_vol_atr={:.3}%>{:.3}%", atr_pct, max_atr),
        );
    }

    (true, format!("maker_ok_atr={:.3}% regime={}", atr_pct, regime.unwrap_or("na")))
}

/// Close a tracked trade on exchange. Returns error if order failed, so caller
/// can keep local state consistent (do NOT mark closed unless exchange close succeeds).
async fn clear_maker_exit_order(
    trade_id: &Uuid,
    maker_exit_orders: &mut HashMap<Uuid, MakerExitState>,
    hl_executor: &Option<HlExecutor>,
) {
    let Some(maker) = maker_exit_orders.remove(trade_id) else {
        return;
    };
    if let Some(hl) = hl_executor {
        if let Err(e) = hl.cancel_order("BTC", maker.oid).await {
            warn!(
                trade_id = %trade_id,
                oid = maker.oid,
                error = %e,
                "Failed to cancel maker exit order"
            );
        }
    }
}

async fn close_trade_on_exchange(
    trade: &ActiveTrade,
    hl_executor: &Option<HlExecutor>,
) -> Result<Option<f64>, String> {
    if let Some(hl) = hl_executor {
        match hl.market_close(trade.symbol.hl_coin()).await {
            Ok(r) if r.success => Ok(r.filled_price),
            Ok(r) => Err(format!("order rejected: {}", r.message)),
            Err(e) => Err(format!("{e:#}")),
        }
    } else {
        Ok(None)
    }
}

fn compute_trade_pnl_with_fees(
    trade: &ActiveTrade,
    exit_price: f64,
    fee_bps_per_side: f64,
) -> (f64, f64, f64) {
    let gross = match trade.direction {
        Direction::Long => (exit_price - trade.entry_price) / trade.entry_price * trade.size_usd,
        Direction::Short => (trade.entry_price - exit_price) / trade.entry_price * trade.size_usd,
    };
    let fee_rate = (fee_bps_per_side.max(0.0) * 2.0) / 10_000.0;
    let fee_usd = trade.size_usd * fee_rate;
    let net = gross - fee_usd;
    (gross, fee_usd, net)
}

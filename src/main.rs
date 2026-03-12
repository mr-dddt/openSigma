mod agent;
mod config;
mod data;
mod execution;
mod journal;
mod signals;
mod tui;
mod types;

use anyhow::Result;
use tokio::sync::mpsc;
use tracing::info;

use crate::config::{Config, Secrets};
use crate::data::hyperliquid::HyperliquidFeed;
use crate::data::news::NewsFeed;
use crate::data::polymarket::PolymarketFeed;
use crate::signals::aggregator::SignalAggregator;
use crate::signals::indicators::Indicators;
use crate::tui::app::App;
use crate::types::*;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "opensigma=info".into()),
        )
        .init();

    info!("openSigma v2 starting...");

    // Load config
    let config = Config::load("config.toml")?;
    let _secrets = Secrets::from_env()?;

    info!(
        capital = config.capital.initial_usd,
        max_trade_pct = config.capital.max_trade_pct,
        max_leverage = config.hyperliquid.max_leverage,
        "Config loaded"
    );

    // Channel for market events (all data feeds → signal engine)
    let (market_tx, mut market_rx) = mpsc::channel::<MarketEvent>(2048);

    // Spawn Hyperliquid WebSocket feed
    let hl_feed = HyperliquidFeed::new(market_tx.clone());
    tokio::spawn(async move { hl_feed.run().await });

    // Spawn Polymarket feed (Phase 1 stub)
    let pm_feed = PolymarketFeed::new(market_tx.clone());
    tokio::spawn(async move { pm_feed.run().await });

    // Spawn news feed (Phase 1 stub)
    let mut news_feed = NewsFeed::new();
    tokio::spawn(async move { news_feed.run().await });

    // Signal aggregator
    let mut aggregator = SignalAggregator::new();
    let config_clone = config.clone();

    // Signal evaluation interval
    let eval_interval =
        tokio::time::Duration::from_secs(config.execution.signal_eval_interval_secs);
    let mut eval_ticker = tokio::time::interval(eval_interval);

    // TUI update channel
    let (tui_tx, mut tui_rx) = mpsc::channel::<TuiUpdate>(256);
    let tui_tx_clone = tui_tx.clone();

    // Spawn the market event processor + signal engine
    tokio::spawn(async move {
        loop {
            tokio::select! {
                Some(event) = market_rx.recv() => {
                    match &event {
                        MarketEvent::Price(tick) => {
                            aggregator.update_price(tick.price);
                            if tick.symbol == Symbol::BTC {
                                let _ = tui_tx_clone.send(TuiUpdate::Price(tick.price)).await;
                            }
                        }
                        MarketEvent::Trade(trade) => {
                            let is_buy = trade.side == Direction::Long;
                            aggregator.indicators.add_trade(trade.size, is_buy);
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
                    let snapshot = aggregator.evaluate(&config_clone);
                    let _ = tui_tx_clone.send(TuiUpdate::Signal(snapshot.clone())).await;

                    // Phase 2: if filter passes, send to LLM gate
                    if snapshot.level != SignalLevel::NoTrade && snapshot.level != SignalLevel::Weak {
                        info!(
                            level = %snapshot.level,
                            score = snapshot.net_score,
                            "Signal detected — would send to LLM (Phase 2)"
                        );
                        let _ = tui_tx_clone.send(TuiUpdate::Log(format!(
                            "[{}] {} (net={}) — LLM gate pending Phase 2",
                            chrono::Utc::now().format("%H:%M:%S"),
                            snapshot.level,
                            snapshot.net_score,
                        ))).await;
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
        // Drain pending TUI updates
        while let Ok(update) = tui_rx.try_recv() {
            match update {
                TuiUpdate::Price(p) => app.update_price(p),
                TuiUpdate::Signal(s) => app.update_signal(s),
                TuiUpdate::Log(l) => app.push_log(l),
            }
        }

        terminal.draw(|frame| app.render_frame(frame))?;

        if crossterm::event::poll(std::time::Duration::from_millis(50))? {
            if let crossterm::event::Event::Key(key) = crossterm::event::read()? {
                if key.kind == crossterm::event::KeyEventKind::Press {
                    match key.code {
                        crossterm::event::KeyCode::Char('q') => break,
                        crossterm::event::KeyCode::Char('k') => {
                            tracing::error!("KILL SWITCH activated");
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
    info!("openSigma v2 shutting down");
    Ok(())
}

use crossterm::ExecutableCommand;

enum TuiUpdate {
    Price(f64),
    Signal(SignalSnapshot),
    Log(String),
}

mod agents;
mod config;
mod data;
mod event_bus;
mod execution;
mod memory;
mod risk;
mod tui;
mod types;

use anyhow::Result;
use tokio::sync::mpsc;
use tracing::info;

use crate::agents::watchdog::WatchDogAgent;
use crate::config::Config;
use crate::data::{coinglass, glassnode, hyperliquid as hl_data, macro_sources};
use crate::event_bus::{llm_filter::LlmFilter, router::TopicRouter, rule_engine::RuleEngine};
use crate::execution::hyperliquid::HyperliquidExecutor;
use crate::memory::store::MemoryStore;
use crate::risk::manager::RiskManager;
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

    info!("openSigma starting...");

    // Load config
    let config = Config::from_env()?;

    // Initialize memory store
    let _memory_store = MemoryStore::new("memory")?;

    // Channel buffer sizes
    const BUS_BUFFER: usize = 1024;
    const AGENT_BUFFER: usize = 256;
    const PROPOSAL_BUFFER: usize = 64;

    // Event bus channels
    let (event_tx, event_rx) = mpsc::channel::<Event>(BUS_BUFFER);

    // Per-agent event channels
    let (watchdog_event_tx, watchdog_event_rx) = mpsc::channel::<Event>(AGENT_BUFFER);
    let (longterm_event_tx, _longterm_event_rx) = mpsc::channel::<Event>(AGENT_BUFFER);
    let (midterm_event_tx, _midterm_event_rx) = mpsc::channel::<Event>(AGENT_BUFFER);
    let (shortterm_event_tx, _shortterm_event_rx) = mpsc::channel::<Event>(AGENT_BUFFER);

    // Proposal channel (agents → WatchDog)
    let (_proposal_tx, proposal_rx) = mpsc::channel::<TradeProposal>(PROPOSAL_BUFFER);

    // Kill switch channel (TUI → WatchDog)
    let (kill_switch_tx, mut kill_switch_rx) = mpsc::channel::<()>(1);

    // Initialize components
    let risk_manager = RiskManager::new(config.risk_limits.clone());
    let executor = HyperliquidExecutor::new(&config.private_key, config.is_mainnet).await?;

    let initial_balance = executor.get_balance().await.unwrap_or(10000.0);
    let _executor = executor; // Keep executor alive for future use
    let mut watchdog = WatchDogAgent::new(risk_manager, proposal_rx, initial_balance);

    // Topic router
    let mut router = TopicRouter::new(
        event_rx,
        watchdog_event_tx,
        longterm_event_tx,
        midterm_event_tx,
        shortterm_event_tx,
    );

    // Data feeds
    let rule_engine = RuleEngine::new(event_tx.clone());
    let mut llm_filter = LlmFilter::new(event_tx.clone(), config.anthropic_api_key.clone());
    let hl_feed = hl_data::HyperliquidFeed::new(event_tx.clone());
    let cg_feed = coinglass::CoinglassFeed::new(event_tx.clone(), config.coinglass_api_key.clone());
    let gn_feed = glassnode::GlassnodeFeed::new(event_tx.clone(), config.glassnode_api_key.clone());
    let macro_feed = macro_sources::MacroFeed::new(event_tx.clone());

    // TUI
    let mut app = App::new(kill_switch_tx);

    info!("All components initialized — spawning tasks");

    // Spawn all async tasks
    tokio::spawn(async move { rule_engine.run().await });
    tokio::spawn(async move { llm_filter.run().await });
    tokio::spawn(async move { router.run().await });
    tokio::spawn(async move { hl_feed.run().await });
    tokio::spawn(async move { cg_feed.run().await });
    tokio::spawn(async move { gn_feed.run().await });
    tokio::spawn(async move { macro_feed.run().await });

    // WatchDog task
    tokio::spawn(async move {
        let memory = MemoryStore::new("memory").expect("memory store");
        watchdog.run(watchdog_event_rx, memory).await
    });

    // Kill switch listener
    tokio::spawn(async move {
        if kill_switch_rx.recv().await.is_some() {
            tracing::error!("KILL SWITCH activated from TUI");
            // TODO: signal WatchDog to close all positions
        }
    });

    // Run TUI on main thread (blocks until quit)
    app.run().await?;

    info!("openSigma shutting down");
    Ok(())
}

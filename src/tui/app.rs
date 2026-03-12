use anyhow::Result;
use crossterm::{
    event::{self, Event as CrosstermEvent, KeyCode, KeyEventKind},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, List, ListItem, Paragraph},
};
use std::io::stdout;
use tokio::sync::mpsc;
use tracing::info;

use crate::types::*;

pub struct App {
    portfolio: Portfolio,
    agent_statuses: Vec<(AgentName, AgentStatus)>,
    trade_log: Vec<String>,
    memory_lines: Vec<String>,
    show_memory: bool,
    should_quit: bool,
    kill_switch_tx: mpsc::Sender<()>,
}

impl App {
    pub fn new(kill_switch_tx: mpsc::Sender<()>) -> Self {
        Self {
            portfolio: Portfolio::default(),
            agent_statuses: vec![
                (AgentName::WatchDog, AgentStatus::Active),
                (AgentName::LongTerm, AgentStatus::Watching),
                (AgentName::MidTerm, AgentStatus::Watching),
                (AgentName::ShortTerm, AgentStatus::Watching),
            ],
            trade_log: Vec::new(),
            memory_lines: Vec::new(),
            show_memory: false,
            should_quit: false,
            kill_switch_tx,
        }
    }

    pub async fn run(&mut self) -> Result<()> {
        enable_raw_mode()?;
        stdout().execute(EnterAlternateScreen)?;
        let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;

        info!("TUI started");

        while !self.should_quit {
            terminal.draw(|frame| self.render(frame))?;

            if event::poll(std::time::Duration::from_millis(100))? {
                if let CrosstermEvent::Key(key) = event::read()? {
                    if key.kind == KeyEventKind::Press {
                        match key.code {
                            KeyCode::Char('q') => self.should_quit = true,
                            KeyCode::Char('k') => {
                                let _ = self.kill_switch_tx.send(()).await;
                            }
                            KeyCode::Char('m') => self.show_memory = !self.show_memory,
                            _ => {}
                        }
                    }
                }
            }
        }

        disable_raw_mode()?;
        stdout().execute(LeaveAlternateScreen)?;
        Ok(())
    }

    fn render(&self, frame: &mut Frame) {
        let chunks = Layout::default()
            .direction(ratatui::layout::Direction::Vertical)
            .constraints([
                Constraint::Length(8),  // portfolio + agent status
                Constraint::Min(10),   // trade log
                Constraint::Length(6), // memory / footer
            ])
            .split(frame.area());

        // Top: Portfolio + Agent Status side by side
        let top_chunks = Layout::default()
            .direction(ratatui::layout::Direction::Horizontal)
            .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
            .split(chunks[0]);

        // Portfolio panel
        let portfolio_text = format!(
            " Equity:   ${:.2}\n Free:     ${:.2}\n PnL:      ${:.2}\n Drawdown: {:.1}%",
            self.portfolio.total_equity_usd,
            self.portfolio.free_cash_usd,
            self.portfolio.realized_pnl,
            self.portfolio.drawdown_pct(),
        );
        let portfolio_block = Paragraph::new(portfolio_text)
            .block(Block::default().borders(Borders::ALL).title(" Portfolio "));
        frame.render_widget(portfolio_block, top_chunks[0]);

        // Agent status panel
        let agent_items: Vec<ListItem> = self
            .agent_statuses
            .iter()
            .map(|(name, status)| ListItem::new(format!("  {:<12} [{}]", name, status)))
            .collect();
        let agent_list = List::new(agent_items)
            .block(Block::default().borders(Borders::ALL).title(" Agents "));
        frame.render_widget(agent_list, top_chunks[1]);

        // Middle: Trade log
        let log_items: Vec<ListItem> = self
            .trade_log
            .iter()
            .rev()
            .take(20)
            .map(|line| ListItem::new(line.as_str()))
            .collect();
        let log_list = List::new(log_items)
            .block(Block::default().borders(Borders::ALL).title(" Trade Log "));
        frame.render_widget(log_list, chunks[1]);

        // Bottom: Memory or keybindings
        let bottom_text = if self.show_memory {
            self.memory_lines
                .iter()
                .rev()
                .take(4)
                .cloned()
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            " [q] quit  [k] kill switch  [m] toggle memory".to_string()
        };
        let bottom_title = if self.show_memory {
            " Memory "
        } else {
            " Keys "
        };
        let bottom_block = Paragraph::new(bottom_text)
            .block(Block::default().borders(Borders::ALL).title(bottom_title));
        frame.render_widget(bottom_block, chunks[2]);
    }

    // --- Update methods called from main loop ---

    pub fn update_portfolio(&mut self, portfolio: Portfolio) {
        self.portfolio = portfolio;
    }

    pub fn update_agent_status(&mut self, name: AgentName, status: AgentStatus) {
        if let Some(entry) = self.agent_statuses.iter_mut().find(|(n, _)| *n == name) {
            entry.1 = status;
        }
    }

    pub fn push_trade_log(&mut self, line: String) {
        self.trade_log.push(line);
    }

    pub fn set_memory(&mut self, content: &str) {
        self.memory_lines = content.lines().map(|l| l.to_string()).collect();
    }
}

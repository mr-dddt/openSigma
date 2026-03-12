use ratatui::{
    prelude::*,
    widgets::{Block, Borders, List, ListItem, Paragraph},
};

use crate::types::*;

pub struct App {
    // State
    pub status: AgentStatus,
    pub btc_price: f64,
    pub latest_signal: Option<SignalSnapshot>,
    pub trade_log: Vec<String>,
    pub equity: f64,
    pub daily_pnl: f64,
}

impl App {
    pub fn new(initial_equity: f64) -> Self {
        Self {
            status: AgentStatus::Scanning,
            btc_price: 0.0,
            latest_signal: None,
            trade_log: Vec::new(),
            equity: initial_equity,
            daily_pnl: 0.0,
        }
    }

    pub fn update_price(&mut self, price: f64) {
        self.btc_price = price;
    }

    pub fn update_signal(&mut self, signal: SignalSnapshot) {
        self.status = if signal.level == SignalLevel::NoTrade || signal.level == SignalLevel::Weak {
            AgentStatus::Scanning
        } else {
            AgentStatus::SignalDetected
        };
        self.latest_signal = Some(signal);
    }

    pub fn push_log(&mut self, line: String) {
        self.trade_log.push(line);
    }

    pub fn render_frame(&self, frame: &mut Frame) {
        let chunks = Layout::default()
            .direction(ratatui::layout::Direction::Vertical)
            .constraints([
                Constraint::Length(7),
                Constraint::Min(10),
                Constraint::Length(3),
            ])
            .split(frame.area());

        // Top: status + signal side by side
        let top_chunks = Layout::default()
            .direction(ratatui::layout::Direction::Horizontal)
            .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
            .split(chunks[0]);

        // Left: portfolio info
        let status_text = format!(
            " BTC:      ${:.2}\n Status:   [{}]\n Equity:   ${:.2}\n Daily PnL: {}{:.2}%",
            self.btc_price,
            self.status,
            self.equity,
            if self.daily_pnl >= 0.0 { "+" } else { "" },
            self.daily_pnl,
        );
        let status_block = Paragraph::new(status_text)
            .block(Block::default().borders(Borders::ALL).title(" openSigma v2 "));
        frame.render_widget(status_block, top_chunks[0]);

        // Right: signal info
        let signal_text = if let Some(ref sig) = self.latest_signal {
            let filter = sig.filter_reason.as_deref().unwrap_or("none");
            format!(
                " Level: {}\n Score: bull={} bear={} net={}\n EMA: 9={:.0} 21={:.0}\n Filter: {}",
                sig.level,
                sig.bull_score,
                sig.bear_score,
                sig.net_score,
                sig.indicators.ema_9.unwrap_or(0.0),
                sig.indicators.ema_21.unwrap_or(0.0),
                filter,
            )
        } else {
            " Waiting for data...".to_string()
        };
        let signal_block = Paragraph::new(signal_text)
            .block(Block::default().borders(Borders::ALL).title(" Signal "));
        frame.render_widget(signal_block, top_chunks[1]);

        // Middle: trade log
        let log_items: Vec<ListItem> = self
            .trade_log
            .iter()
            .rev()
            .take(20)
            .map(|line| ListItem::new(line.as_str()))
            .collect();
        let log_list = List::new(log_items)
            .block(Block::default().borders(Borders::ALL).title(" Log "));
        frame.render_widget(log_list, chunks[1]);

        // Bottom: keybindings
        let footer = Paragraph::new(" [q] quit  [k] kill switch")
            .block(Block::default().borders(Borders::ALL));
        frame.render_widget(footer, chunks[2]);
    }
}

use ratatui::{
    prelude::{Color, Constraint, Frame, Layout, Line, Rect, Span, Style, Stylize},
    widgets::{Block, Borders, List, ListItem, Paragraph},
};

use crate::types::{
    AgentStatus, Direction, SignalLevel, SignalSnapshot,
};

// TUI-specific display types — sourced from HL queries, not internal calculations

#[derive(Debug, Clone)]
pub struct PositionInfo {
    pub coin: String,
    pub direction: Direction,
    pub entry_price: f64,
    pub notional: f64,     // |size| * entry_price
    pub leverage: u8,
    pub unrealized_pnl: f64, // from HL directly
}

#[derive(Debug, Clone, Default)]
pub struct PerformanceStats {
    pub total_trades: u64,
    pub win_rate: f64,
    pub total_pnl: f64,
    pub streak: i32,
}

#[derive(Debug, Clone, Default)]
pub struct ExchangeBalances {
    pub hl_equity: f64,
    pub hl_available: f64,
}

pub struct App {
    // State
    pub status: AgentStatus,
    pub btc_price: f64,
    pub latest_signal: Option<SignalSnapshot>,
    pub trade_log: Vec<String>,
    pub initial_equity: f64,
    pub equity: f64,
    pub daily_pnl: f64,
    pub max_positions: u32,
    // Phase 3 additions
    pub positions: Vec<PositionInfo>,
    pub stats: PerformanceStats,
    pub balances: ExchangeBalances,
}

impl App {
    pub fn new(initial_equity: f64, max_positions: u32) -> Self {
        Self {
            status: AgentStatus::Scanning,
            btc_price: 0.0,
            latest_signal: None,
            trade_log: Vec::new(),
            initial_equity,
            equity: initial_equity,
            daily_pnl: 0.0,
            max_positions,
            positions: Vec::new(),
            stats: PerformanceStats::default(),
            balances: ExchangeBalances::default(),
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

    pub fn update_positions(&mut self, positions: Vec<PositionInfo>) {
        self.positions = positions;
        if !self.positions.is_empty() {
            self.status = AgentStatus::InPosition;
        }
    }

    pub fn update_stats(&mut self, stats: PerformanceStats) {
        self.stats = stats;
    }

    pub fn update_balances(&mut self, balances: ExchangeBalances) {
        self.balances = balances;
        self.equity = self.balances.hl_equity;
        if self.initial_equity > 0.0 {
            self.daily_pnl = ((self.equity - self.initial_equity) / self.initial_equity) * 100.0;
        }
    }

    pub fn render_frame(&self, frame: &mut Frame) {
        let pos_height = (self.positions.len() as u16 + 2).max(4).min(14);
        let chunks = Layout::default()
            .direction(ratatui::layout::Direction::Vertical)
            .constraints([
                Constraint::Length(7),         // top: status + signal
                Constraint::Length(pos_height), // positions: grows with count
                Constraint::Min(6),            // log
                Constraint::Length(3),          // footer: stats + keys
            ])
            .split(frame.area());

        self.render_top(frame, chunks[0]);
        self.render_positions(frame, chunks[1]);
        self.render_log(frame, chunks[2]);
        self.render_footer(frame, chunks[3]);
    }

    fn render_top(&self, frame: &mut Frame, area: Rect) {
        let top_chunks = Layout::default()
            .direction(ratatui::layout::Direction::Horizontal)
            .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
            .split(area);

        // Left: portfolio info
        let pnl_color = if self.daily_pnl >= 0.0 { Color::Green } else { Color::Red };
        let pnl_sign = if self.daily_pnl >= 0.0 { "+" } else { "" };

        let status_lines = vec![
            Line::from(vec![
                Span::styled(" BTC:", Style::default().fg(Color::Cyan)),
                Span::styled(format!("${:.0}", self.btc_price), Style::default().fg(Color::White).bold()),
                Span::styled(format!("  [{}]", self.status), Style::default().fg(status_color(self.status))),
            ]),
            Line::from(vec![
                Span::styled(" HL:   ", Style::default().fg(Color::Cyan)),
                Span::styled(format!("${:.2}", self.balances.hl_equity), Style::default().fg(Color::White)),
                Span::styled("  Free: ", Style::default().fg(Color::Cyan)),
                Span::styled(format!("${:.2}", self.balances.hl_available), Style::default().fg(Color::White)),
            ]),
            Line::from(vec![
                Span::styled(" Total:", Style::default().fg(Color::Cyan)),
                Span::styled(format!(" ${:.2}", self.equity), Style::default().fg(Color::White).bold()),
            ]),
            Line::from(vec![
                Span::styled(" PnL:  ", Style::default().fg(Color::Cyan)),
                Span::styled(
                    format!(" {pnl_sign}{:.2}%", self.daily_pnl),
                    Style::default().fg(pnl_color),
                ),
            ]),
        ];

        let status_block = Paragraph::new(status_lines)
            .block(Block::default().borders(Borders::ALL).title(" openSigma v1 ").border_style(Style::default().fg(Color::Cyan)));
        frame.render_widget(status_block, top_chunks[0]);

        // Right: signal info with all indicators
        let signal_lines = if let Some(ref sig) = self.latest_signal {
            let ind = &sig.indicators;
            let filter = sig.filter_reason.as_deref().unwrap_or("none");
            let level_color = signal_level_color(sig.level);

            vec![
                Line::from(vec![
                    Span::styled(format!(" {}", sig.level), Style::default().fg(level_color).bold()),
                    Span::raw(format!(" (net={:+})  ", sig.net_score)),
                    Span::styled(format!("bull={} bear={}", sig.bull_score, sig.bear_score), Style::default().fg(Color::DarkGray)),
                ]),
                Line::from(vec![
                    Span::styled(" EMA:", Style::default().fg(Color::Cyan)),
                    Span::raw(format!(" 9={:.0} 21={:.0}", ind.ema_9.unwrap_or(0.0), ind.ema_21.unwrap_or(0.0))),
                    Span::styled("  RSI:", Style::default().fg(Color::Cyan)),
                    Span::raw(format!(" {:.1}", ind.rsi_14.unwrap_or(0.0))),
                    Span::styled("  StochRSI:", Style::default().fg(Color::Cyan)),
                    Span::raw(format!(" {:.1}", ind.stoch_rsi.unwrap_or(0.0))),
                ]),
                Line::from(vec![
                    Span::styled(" CVD:", Style::default().fg(Color::Cyan)),
                    Span::raw(format!(" {:+.2}", ind.cvd.unwrap_or(0.0))),
                    Span::styled("  OB:", Style::default().fg(Color::Cyan)),
                    Span::raw(format!(" {:.2}", ind.ob_imbalance.unwrap_or(1.0))),
                    Span::styled("  ATR%:", Style::default().fg(Color::Cyan)),
                    Span::raw(format!(" {:.3}", ind.atr_pct.unwrap_or(0.0))),
                    Span::styled("  BB:", Style::default().fg(Color::Cyan)),
                    Span::styled(
                        format!(" {}", bb_label(ind.bb_squeeze, ind.bb_position.unwrap_or(0.0))),
                        Style::default().fg(bb_color(ind.bb_squeeze, ind.bb_position.unwrap_or(0.0))),
                    ),
                ]),
                Line::from(vec![
                    Span::styled(" Filter: ", Style::default().fg(Color::DarkGray)),
                    Span::styled(filter, Style::default().fg(if filter == "none" { Color::DarkGray } else { Color::Yellow })),
                ]),
            ]
        } else {
            vec![Line::from(Span::styled(" Waiting for data...", Style::default().fg(Color::DarkGray)))]
        };

        let signal_block = Paragraph::new(signal_lines)
            .block(Block::default().borders(Borders::ALL).title(" Signal ").border_style(Style::default().fg(Color::Cyan)));
        frame.render_widget(signal_block, top_chunks[1]);
    }

    fn render_positions(&self, frame: &mut Frame, area: Rect) {
        let title = format!(" Positions [{}/{}] ", self.positions.len(), self.max_positions);
        let items: Vec<ListItem> = if self.positions.is_empty() {
            vec![ListItem::new(Span::styled("  No active positions", Style::default().fg(Color::DarkGray)))]
        } else {
            self.positions.iter().map(|p| {
                let pnl_color = if p.unrealized_pnl >= 0.0 { Color::Green } else { Color::Red };
                let dir_color = if p.direction == Direction::Long { Color::Green } else { Color::Red };

                ListItem::new(Line::from(vec![
                    Span::styled(format!(" {} ", p.direction), Style::default().fg(dir_color).bold()),
                    Span::raw(format!("{} ${:.0} @{:.0} lev={}  ", p.coin, p.notional, p.entry_price, p.leverage)),
                    Span::styled("PnL: ", Style::default().fg(Color::Cyan)),
                    Span::styled(
                        format!("${:+.2}", p.unrealized_pnl),
                        Style::default().fg(pnl_color).bold(),
                    ),
                ]))
            }).collect()
        };

        let positions_block = List::new(items)
            .block(Block::default().borders(Borders::ALL).title(title).border_style(Style::default().fg(Color::Cyan)));
        frame.render_widget(positions_block, area);
    }

    fn render_log(&self, frame: &mut Frame, area: Rect) {
        let log_items: Vec<ListItem> = self
            .trade_log
            .iter()
            .rev()
            .take(area.height.saturating_sub(2) as usize)
            .map(|line| {
                let color = if line.contains("EXECUTE") {
                    Color::Green
                } else if line.contains("SKIP") || line.contains("SECOND_LOOK") {
                    Color::Yellow
                } else if line.contains("KILL") || line.contains("Risk") {
                    Color::Red
                } else if line.contains("Report") || line.contains("Tune") {
                    Color::Cyan
                } else {
                    Color::White
                };
                ListItem::new(Span::styled(line.as_str(), Style::default().fg(color)))
            })
            .collect();

        let log_list = List::new(log_items)
            .block(Block::default().borders(Borders::ALL).title(" Log ").border_style(Style::default().fg(Color::Cyan)));
        frame.render_widget(log_list, area);
    }

    fn render_footer(&self, frame: &mut Frame, area: Rect) {
        let streak_str = if self.stats.streak > 0 {
            format!("W{}", self.stats.streak)
        } else if self.stats.streak < 0 {
            format!("L{}", self.stats.streak.abs())
        } else {
            "—".to_string()
        };
        let streak_color = if self.stats.streak > 0 {
            Color::Green
        } else if self.stats.streak < 0 {
            Color::Red
        } else {
            Color::DarkGray
        };

        let pnl_color = if self.stats.total_pnl >= 0.0 { Color::Green } else { Color::Red };

        let footer_line = Line::from(vec![
            Span::styled(" Trades: ", Style::default().fg(Color::Cyan)),
            Span::raw(format!("{}", self.stats.total_trades)),
            Span::styled("  Win: ", Style::default().fg(Color::Cyan)),
            Span::raw(format!("{:.1}%", self.stats.win_rate * 100.0)),
            Span::styled("  PnL: ", Style::default().fg(Color::Cyan)),
            Span::styled(format!("${:+.2}", self.stats.total_pnl), Style::default().fg(pnl_color)),
            Span::styled("  Streak: ", Style::default().fg(Color::Cyan)),
            Span::styled(streak_str, Style::default().fg(streak_color)),
            Span::styled("    [q] quit  [k] kill switch", Style::default().fg(Color::DarkGray)),
        ]);

        let footer = Paragraph::new(footer_line)
            .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::Cyan)));
        frame.render_widget(footer, area);
    }
}

fn status_color(status: AgentStatus) -> Color {
    match status {
        AgentStatus::Scanning => Color::DarkGray,
        AgentStatus::SignalDetected => Color::Yellow,
        AgentStatus::WaitingLlm => Color::Yellow,
        AgentStatus::Executing => Color::Green,
        AgentStatus::InPosition => Color::Green,
        AgentStatus::SecondLook => Color::Yellow,
        AgentStatus::Paused => Color::DarkGray,
        AgentStatus::KillSwitch => Color::Red,
    }
}

fn signal_level_color(level: SignalLevel) -> Color {
    match level {
        SignalLevel::StrongLong => Color::Magenta,
        SignalLevel::LeanLong => Color::Yellow,
        SignalLevel::Weak => Color::DarkGray,
        SignalLevel::LeanShort => Color::Yellow,
        SignalLevel::StrongShort => Color::Magenta,
        SignalLevel::NoTrade => Color::DarkGray,
    }
}

fn bb_label(squeeze: bool, pos: f64) -> String {
    if squeeze {
        if pos <= -0.7 { "SQ↓".into() }
        else if pos >= 0.7 { "SQ↑".into() }
        else { "SQ".into() }
    } else if pos > 1.0 {
        "BRK↑".into()
    } else if pos < -1.0 {
        "BRK↓".into()
    } else {
        format!("{:+.1}", pos)
    }
}

fn bb_color(squeeze: bool, pos: f64) -> Color {
    if squeeze {
        if pos <= -0.7 { Color::Green }
        else if pos >= 0.7 { Color::Red }
        else { Color::Yellow }
    } else if pos.abs() > 1.0 {
        Color::Magenta
    } else {
        Color::DarkGray
    }
}

use std::collections::HashMap;

use anyhow::{Context, Result};
use serde::Deserialize;
use tracing::{info, warn};
use uuid::Uuid;

use crate::agent::llm_client::LlmClient;
use crate::journal::memory::MemoryManager;
use crate::types::{PlayType, TradeRecord};

const REPORT_SYSTEM_PROMPT: &str = r#"You are openSigma's trade analyst. Given a batch of recent trades and performance metrics, identify patterns and suggest improvements.

Respond with ONLY a valid JSON object:
{
  "patterns": ["pattern 1", "pattern 2"],
  "memory_rules": ["rule to add to memory (only high-confidence observations)"],
  "summary": "1-2 sentence summary of this batch"
}

Rules:
- Identify 2-5 patterns from the trade data (winning setups, losing patterns, session effects, etc.)
- Only add memory_rules you are highly confident about (0-3 max)
- Memory rules should be actionable (e.g., "SecondLook entries win at 75%+ after 3 consecutive losses")
- Keep summary concise — it will be shown in TUI and Telegram
- If no clear patterns, say so honestly"#;

/// 20-trade report generator with LLM analysis and memory integration.
pub struct Reporter {
    llm_client: LlmClient,
    report_dir: String,
}

#[allow(dead_code)]
pub struct ReportResult {
    pub report_path: String,
    pub summary: String,
    pub memory_additions: Vec<String>,
}

#[allow(dead_code)]
struct ReportMetrics {
    total: usize,
    wins: usize,
    losses: usize,
    win_rate: f64,
    net_pnl: f64,
    avg_pnl: f64,
    best_trade: Option<(Uuid, f64)>,
    worst_trade: Option<(Uuid, f64)>,
    by_play_type: HashMap<PlayType, (usize, usize, f64)>, // (count, wins, pnl)
    by_exit_reason: HashMap<String, usize>,
    avg_duration_secs: f64,
}

#[derive(Deserialize)]
struct ReportAnalysis {
    #[serde(default)]
    patterns: Vec<String>,
    #[serde(default)]
    memory_rules: Vec<String>,
    #[serde(default = "default_summary")]
    summary: String,
}

fn default_summary() -> String {
    "No analysis available.".to_string()
}

impl Reporter {
    pub fn new(api_key: &str, model: &str) -> Self {
        let llm_client = LlmClient::new(
            api_key.to_string(),
            model.to_string(),
            10_000, // longer timeout for report analysis
        );
        Self {
            llm_client,
            report_dir: "data/reports".to_string(),
        }
    }

    /// Generate a report from a batch of trades, update memory, return result.
    pub async fn generate(
        &self,
        trades: &[TradeRecord],
        memory: &MemoryManager,
    ) -> Result<ReportResult> {
        if trades.is_empty() {
            anyhow::bail!("No trades to report on");
        }

        let metrics = compute_metrics(trades);
        let report_num = self.next_report_number();

        // Build LLM context
        let context = self.build_context(&metrics, trades, memory);

        // Call LLM for analysis
        let analysis = match self.llm_client.prompt(REPORT_SYSTEM_PROMPT, &context, 1024).await {
            Ok(text) => parse_analysis(&text),
            Err(e) => {
                warn!("Report LLM analysis failed: {e:#}");
                ReportAnalysis {
                    patterns: vec!["LLM analysis unavailable".to_string()],
                    memory_rules: vec![],
                    summary: format!("Batch of {} trades, {:.0}% win rate, PnL ${:.2}", metrics.total, metrics.win_rate * 100.0, metrics.net_pnl),
                }
            }
        };

        // Write markdown report
        std::fs::create_dir_all(&self.report_dir).ok();
        let report_path = format!("{}/report_{:03}.md", self.report_dir, report_num);
        let report_content = self.format_report(report_num, &metrics, &analysis);
        std::fs::write(&report_path, &report_content)
            .with_context(|| format!("Failed to write report to {report_path}"))?;

        // Append memory rules
        let memory_additions = analysis.memory_rules.clone();
        if !memory_additions.is_empty() {
            let memory_block = format!(
                "\n## Report #{} — {}\n{}",
                report_num,
                chrono::Utc::now().format("%Y-%m-%d %H:%M UTC"),
                memory_additions
                    .iter()
                    .map(|r| format!("- {r}"))
                    .collect::<Vec<_>>()
                    .join("\n"),
            );
            memory.append(&memory_block)?;
        }

        info!(path = %report_path, "Report generated");

        Ok(ReportResult {
            report_path,
            summary: analysis.summary,
            memory_additions,
        })
    }

    fn next_report_number(&self) -> u32 {
        let count = std::fs::read_dir(&self.report_dir)
            .map(|entries| entries.filter_map(|e| e.ok()).count())
            .unwrap_or(0);
        (count + 1) as u32
    }

    fn build_context(
        &self,
        metrics: &ReportMetrics,
        trades: &[TradeRecord],
        memory: &MemoryManager,
    ) -> String {
        let trades_detail: String = trades
            .iter()
            .map(|t| {
                format!(
                    "  #{} {} {} pnl=${:.2} level={} score={} exit={} dur={}s",
                    t.id,
                    t.direction,
                    t.play_type,
                    t.pnl_usd.unwrap_or(0.0),
                    t.signal_level,
                    t.signal_score,
                    t.exit_reason.as_deref().unwrap_or("?"),
                    t.duration_secs.unwrap_or(0),
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        let play_type_breakdown: String = metrics
            .by_play_type
            .iter()
            .map(|(pt, (count, wins, pnl))| {
                format!("  {pt}: {count} trades, {wins} wins, PnL ${pnl:.2}")
            })
            .collect::<Vec<_>>()
            .join("\n");

        let exit_breakdown: String = metrics
            .by_exit_reason
            .iter()
            .map(|(reason, count)| format!("  {reason}: {count}"))
            .collect::<Vec<_>>()
            .join("\n");

        format!(
            "Trade batch ({} trades):\n{}\n\n\
             Metrics:\n\
             - Win rate: {:.1}% ({}/{})\n\
             - Net PnL: ${:.2}\n\
             - Avg PnL: ${:.2}\n\
             - Best: {} (${:.2})\n\
             - Worst: {} (${:.2})\n\
             - Avg duration: {:.0}s\n\n\
             By play type:\n{}\n\n\
             By exit reason:\n{}\n\n\
             Current memory:\n{}",
            metrics.total,
            trades_detail,
            metrics.win_rate * 100.0,
            metrics.wins,
            metrics.total,
            metrics.net_pnl,
            metrics.avg_pnl,
            metrics.best_trade.map_or("none".to_string(), |(id, _)| id.to_string()),
            metrics.best_trade.map_or(0.0, |(_, pnl)| pnl),
            metrics.worst_trade.map_or("none".to_string(), |(id, _)| id.to_string()),
            metrics.worst_trade.map_or(0.0, |(_, pnl)| pnl),
            metrics.avg_duration_secs,
            play_type_breakdown,
            exit_breakdown,
            memory.recent_summary(),
        )
    }

    fn format_report(
        &self,
        num: u32,
        metrics: &ReportMetrics,
        analysis: &ReportAnalysis,
    ) -> String {
        let patterns = if analysis.patterns.is_empty() {
            "No clear patterns detected.".to_string()
        } else {
            analysis
                .patterns
                .iter()
                .enumerate()
                .map(|(i, p)| format!("{}. {p}", i + 1))
                .collect::<Vec<_>>()
                .join("\n")
        };

        let memory_rules = if analysis.memory_rules.is_empty() {
            "No new rules added.".to_string()
        } else {
            analysis
                .memory_rules
                .iter()
                .map(|r| format!("- {r}"))
                .collect::<Vec<_>>()
                .join("\n")
        };

        let play_type_table: String = metrics
            .by_play_type
            .iter()
            .map(|(pt, (count, wins, pnl))| {
                let wr = if *count > 0 {
                    (*wins as f64 / *count as f64) * 100.0
                } else {
                    0.0
                };
                format!("| {pt} | {count} | {wr:.0}% | ${pnl:.2} |")
            })
            .collect::<Vec<_>>()
            .join("\n");

        format!(
            "# openSigma Report #{num} — {ts}\n\n\
             ## Performance\n\
             - Trades: {total} | Win Rate: {wr:.0}% ({wins}/{total})\n\
             - Net PnL: ${net:.2} | Avg PnL: ${avg:.2}\n\
             - Best: {best} | Worst: {worst}\n\
             - Avg Duration: {dur:.0}s\n\n\
             ## By Play Type\n\
             | Type | Count | Win% | Net PnL |\n\
             |------|-------|------|---------|\n\
             {play_types}\n\n\
             ## Patterns Detected\n\
             {patterns}\n\n\
             ## Memory Updates\n\
             {memory_rules}\n\n\
             ## Summary\n\
             {summary}\n",
            ts = chrono::Utc::now().format("%Y-%m-%d %H:%M UTC"),
            total = metrics.total,
            wr = metrics.win_rate * 100.0,
            wins = metrics.wins,
            net = metrics.net_pnl,
            avg = metrics.avg_pnl,
            best = metrics.best_trade.map_or("—".to_string(), |(id, pnl)| format!("#{id} ${pnl:.2}")),
            worst = metrics.worst_trade.map_or("—".to_string(), |(id, pnl)| format!("#{id} ${pnl:.2}")),
            dur = metrics.avg_duration_secs,
            play_types = play_type_table,
            patterns = patterns,
            memory_rules = memory_rules,
            summary = analysis.summary,
        )
    }
}

/// Pure function: compute metrics from a batch of trades.
fn compute_metrics(trades: &[TradeRecord]) -> ReportMetrics {
    let total = trades.len();
    let mut wins = 0usize;
    let mut losses = 0usize;
    let mut net_pnl = 0.0f64;
    let mut best_trade: Option<(Uuid, f64)> = None;
    let mut worst_trade: Option<(Uuid, f64)> = None;
    let mut by_play_type: HashMap<PlayType, (usize, usize, f64)> = HashMap::new();
    let mut by_exit_reason: HashMap<String, usize> = HashMap::new();
    let mut total_duration = 0u64;

    for t in trades {
        let pnl = t.pnl_usd.unwrap_or(0.0);
        net_pnl += pnl;

        if pnl >= 0.0 {
            wins += 1;
        } else {
            losses += 1;
        }

        match &best_trade {
            None => best_trade = Some((t.id, pnl)),
            Some((_, best_pnl)) if pnl > *best_pnl => best_trade = Some((t.id, pnl)),
            _ => {}
        }
        match &worst_trade {
            None => worst_trade = Some((t.id, pnl)),
            Some((_, worst_pnl)) if pnl < *worst_pnl => worst_trade = Some((t.id, pnl)),
            _ => {}
        }

        let entry = by_play_type.entry(t.play_type).or_insert((0, 0, 0.0));
        entry.0 += 1;
        if pnl >= 0.0 {
            entry.1 += 1;
        }
        entry.2 += pnl;

        if let Some(ref reason) = t.exit_reason {
            *by_exit_reason.entry(reason.clone()).or_insert(0) += 1;
        }

        total_duration += t.duration_secs.unwrap_or(0);
    }

    let win_rate = if total > 0 { wins as f64 / total as f64 } else { 0.0 };
    let avg_pnl = if total > 0 { net_pnl / total as f64 } else { 0.0 };
    let avg_duration_secs = if total > 0 { total_duration as f64 / total as f64 } else { 0.0 };

    ReportMetrics {
        total,
        wins,
        losses,
        win_rate,
        net_pnl,
        avg_pnl,
        best_trade,
        worst_trade,
        by_play_type,
        by_exit_reason,
        avg_duration_secs,
    }
}

/// Parse LLM response into ReportAnalysis, with fallback.
fn parse_analysis(text: &str) -> ReportAnalysis {
    let trimmed = text.trim();

    // Try extracting JSON from markdown blocks
    let json_str = if let Some(start) = trimmed.find("```json") {
        let s = start + 7;
        if let Some(end) = trimmed[s..].find("```") {
            &trimmed[s..s + end]
        } else {
            trimmed
        }
    } else if let Some(start) = trimmed.find('{') {
        if let Some(end) = trimmed.rfind('}') {
            &trimmed[start..=end]
        } else {
            trimmed
        }
    } else {
        trimmed
    };

    serde_json::from_str::<ReportAnalysis>(json_str.trim()).unwrap_or_else(|_| {
        warn!("Failed to parse report analysis JSON, using fallback");
        ReportAnalysis {
            patterns: vec![],
            memory_rules: vec![],
            summary: text.chars().take(200).collect(),
        }
    })
}

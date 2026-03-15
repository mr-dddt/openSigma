use std::collections::HashMap;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};
use uuid::Uuid;

use crate::agent::llm_client::LlmClient;
use crate::journal::memory::MemoryManager;
use crate::types::{PlayType, TradeRecord};

const REPORT_SYSTEM_PROMPT: &str = r#"You are openSigma's trade analyst. Given a batch of recent trades with detailed entry conditions (RSI, CVD, OB, ATR%, BB position), performance metrics, and current signal params, identify patterns and suggest improvements.

Respond with ONLY a valid JSON object:
{
  "patterns": ["pattern 1", "pattern 2"],
  "memory_rules": ["rule to add to memory (only high-confidence, actionable observations)"],
  "summary": "1-2 sentence summary of this batch",
  "param_adjustments": [{"param": "strong_threshold", "value": 4, "reason": "brief reason"}]
}

Rules:
- Use the per-trade entry conditions (rsi, cvd, ob, atr_pct, bb_position) to find what setups work/fail. Refer to concrete values (e.g. "Longs when RSI<35 and CVD>0 won 80%").
- memory_rules: 0-3 max, actionable (e.g. "When RSI<30 and CVD>0, lean_long wins often — consider lower lean_threshold" or "Strong entries after 3 losses tend to fail").
- param_adjustments: optional. Suggest 0-3 signal param changes when data supports it. Supported params: strong_threshold (int), lean_threshold (int), min_atr_pct (float), rsi_oversold (float), rsi_overbought (float). reason must cite the condition (e.g. "strong_threshold=5 too strict when RSI<35; wins at score 4").
- Keep summary concise — shown in TUI and Telegram.
- If no clear patterns or no confident param change, use empty arrays."#;

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
    pub param_adjustments: Vec<ParamAdjustment>,
}

/// Single signal-param change suggested by report LLM (applied to tuned_signals).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParamAdjustment {
    pub param: String,
    #[serde(rename = "value")]
    pub value: serde_json::Number,
    pub reason: String,
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
    #[serde(default)]
    param_adjustments: Vec<ParamAdjustment>,
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
    /// `current_signal_params`: human-readable string of signal params for LLM (e.g. strong_threshold=5).
    pub async fn generate(
        &self,
        trades: &[TradeRecord],
        memory: &MemoryManager,
        current_signal_params: &str,
    ) -> Result<ReportResult> {
        if trades.is_empty() {
            anyhow::bail!("No trades to report on");
        }

        let metrics = compute_metrics(trades);
        let report_num = self.next_report_number();

        // Build LLM context
        let context = self.build_context(&metrics, trades, memory, current_signal_params);

        // Call LLM for analysis
        let analysis = match self.llm_client.prompt(REPORT_SYSTEM_PROMPT, &context, 1024).await {
            Ok(text) => parse_analysis(&text),
            Err(e) => {
                warn!("Report LLM analysis failed: {e:#}");
                ReportAnalysis {
                    patterns: vec!["LLM analysis unavailable".to_string()],
                    memory_rules: vec![],
                    summary: format!("Batch of {} trades, {:.0}% win rate, PnL ${:.2}", metrics.total, metrics.win_rate * 100.0, metrics.net_pnl),
                    param_adjustments: vec![],
                }
            }
        };

        // Write markdown report
        std::fs::create_dir_all(&self.report_dir).ok();
        let report_path = format!("{}/report_{:03}.md", self.report_dir, report_num);
        let report_content =
            self.format_report(report_num, trades, &metrics, &analysis, current_signal_params);
        std::fs::write(&report_path, &report_content)
            .with_context(|| format!("Failed to write report to {report_path}"))?;

        // Append detailed memory block for every review.
        let memory_additions = analysis.memory_rules.clone();
        let rules_block = if memory_additions.is_empty() {
            "- No new memory rules.".to_string()
        } else {
            memory_additions
                .iter()
                .map(|r| format!("- {r}"))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let patterns_block = if analysis.patterns.is_empty() {
            "- No clear pattern this round.".to_string()
        } else {
            analysis
                .patterns
                .iter()
                .map(|p| format!("- {p}"))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let tune_block = if analysis.param_adjustments.is_empty() {
            "- No param changes suggested.".to_string()
        } else {
            analysis
                .param_adjustments
                .iter()
                .map(|a| format!("- {} = {} ({})", a.param, a.value, a.reason))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let worked_block = trades
            .iter()
            .filter(|t| t.pnl_usd.unwrap_or(0.0) >= 0.0)
            .take(3)
            .map(|t| {
                format!(
                    "- {} score={} rsi={} cvd={} ob={} atr%={} bb={} -> {} (${:.2})",
                    t.signal_level,
                    t.signal_score,
                    t.entry_rsi.map_or("n/a".to_string(), |v| format!("{v:.1}")),
                    t.entry_cvd.map_or("n/a".to_string(), |v| format!("{v:.0}")),
                    t.entry_ob.map_or("n/a".to_string(), |v| format!("{v:.2}")),
                    t.entry_atr_pct.map_or("n/a".to_string(), |v| format!("{v:.2}")),
                    t.entry_bb_position.map_or("n/a".to_string(), |v| format!("{v:.2}")),
                    t.exit_reason.as_deref().unwrap_or("?"),
                    t.pnl_usd.unwrap_or(0.0),
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        let failed_block = trades
            .iter()
            .filter(|t| t.pnl_usd.unwrap_or(0.0) < 0.0)
            .take(3)
            .map(|t| {
                format!(
                    "- {} score={} rsi={} cvd={} ob={} atr%={} bb={} -> {} (${:.2})",
                    t.signal_level,
                    t.signal_score,
                    t.entry_rsi.map_or("n/a".to_string(), |v| format!("{v:.1}")),
                    t.entry_cvd.map_or("n/a".to_string(), |v| format!("{v:.0}")),
                    t.entry_ob.map_or("n/a".to_string(), |v| format!("{v:.2}")),
                    t.entry_atr_pct.map_or("n/a".to_string(), |v| format!("{v:.2}")),
                    t.entry_bb_position.map_or("n/a".to_string(), |v| format!("{v:.2}")),
                    t.exit_reason.as_deref().unwrap_or("?"),
                    t.pnl_usd.unwrap_or(0.0),
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        let memory_block = format!(
            "\n## Report #{} — {}\n### Summary\n{}\n\n### Parameter Snapshot\n{}\n\n### Worked Conditions (examples)\n{}\n\n### Failed Conditions (examples)\n{}\n\n### Patterns\n{}\n\n### Memory Rules\n{}\n\n### Param Tuning Suggestions\n{}",
            report_num,
            chrono::Utc::now().format("%Y-%m-%d %H:%M UTC"),
            analysis.summary,
            current_signal_params,
            if worked_block.is_empty() { "- none".to_string() } else { worked_block },
            if failed_block.is_empty() { "- none".to_string() } else { failed_block },
            patterns_block,
            rules_block,
            tune_block,
        );
        memory.append(&memory_block)?;

        info!(path = %report_path, "Report generated");

        Ok(ReportResult {
            report_path,
            summary: analysis.summary,
            memory_additions,
            param_adjustments: analysis.param_adjustments.clone(),
        })
    }

    fn next_report_number(&self) -> u32 {
        let count = std::fs::read_dir(&self.report_dir)
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .filter(|e| e.file_name().to_string_lossy().starts_with("report_") && e.file_name().to_string_lossy().ends_with(".md"))
                    .count()
            })
            .unwrap_or(0);
        (count + 1) as u32
    }

    fn build_context(
        &self,
        metrics: &ReportMetrics,
        trades: &[TradeRecord],
        memory: &MemoryManager,
        current_signal_params: &str,
    ) -> String {
        let trades_detail: String = trades
            .iter()
            .map(|t| {
                let outcome = if t.pnl_usd.map_or(false, |p| p >= 0.0) {
                    "win"
                } else {
                    "loss"
                };
                let rsi = t.entry_rsi.map_or("n/a".to_string(), |v| format!("{v:.1}"));
                let cvd = t.entry_cvd.map_or("n/a".to_string(), |v| format!("{v:.0}"));
                let ob = t.entry_ob.map_or("n/a".to_string(), |v| format!("{v:.2}"));
                let atr = t.entry_atr_pct.map_or("n/a".to_string(), |v| format!("{v:.2}"));
                let bb = t.entry_bb_position.map_or("n/a".to_string(), |v| format!("{v:.2}"));
                format!(
                    "  #{} {} {} | rsi={} cvd={} ob={} atr%={} bb_pos={} | level={} score={} | exit={} {} | dur={}s | {}",
                    t.id,
                    t.direction,
                    t.play_type,
                    rsi,
                    cvd,
                    ob,
                    atr,
                    bb,
                    t.signal_level,
                    t.signal_score,
                    t.exit_reason.as_deref().unwrap_or("?"),
                    outcome,
                    t.duration_secs.unwrap_or(0),
                    t.llm_reasoning.chars().take(80).collect::<String>(),
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
            "Trade batch ({} trades) — each line: entry conditions (rsi, cvd, ob, atr%%, bb_pos), level/score, exit reason, outcome, duration, LLM reasoning snippet:\n{}\n\n\
             Metrics:\n\
             - Win rate: {:.1}% ({}/{})\n\
             - Net PnL: ${:.2}\n\
             - Avg PnL: ${:.2}\n\
             - Best: {} (${:.2})\n\
             - Worst: {} (${:.2})\n\
             - Avg duration: {:.0}s\n\n\
             By play type:\n{}\n\n\
             By exit reason:\n{}\n\n\
             Current signal params (for param_adjustments):\n{}\n\n\
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
            current_signal_params,
            memory.recent_summary(),
        )
    }

    fn format_report(
        &self,
        num: u32,
        trades: &[TradeRecord],
        metrics: &ReportMetrics,
        analysis: &ReportAnalysis,
        current_signal_params: &str,
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

        let param_section = if analysis.param_adjustments.is_empty() {
            "None suggested.".to_string()
        } else {
            analysis
                .param_adjustments
                .iter()
                .map(|a| format!("- {} = {} — {}", a.param, a.value, a.reason))
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

        let condition_table: String = trades
            .iter()
            .map(|t| {
                let outcome = if t.pnl_usd.map_or(false, |p| p >= 0.0) {
                    "work"
                } else {
                    "fail"
                };
                format!(
                    "| {} | {} | {} | {:.2} | {} | {} | {} | {} | {} | {} | {} | {} | {} |",
                    t.id,
                    t.direction,
                    t.play_type,
                    t.pnl_usd.unwrap_or(0.0),
                    t.signal_level,
                    t.signal_score,
                    t.entry_rsi.map_or("n/a".to_string(), |v| format!("{v:.1}")),
                    t.entry_cvd.map_or("n/a".to_string(), |v| format!("{v:.0}")),
                    t.entry_ob.map_or("n/a".to_string(), |v| format!("{v:.2}")),
                    t.entry_atr_pct.map_or("n/a".to_string(), |v| format!("{v:.2}")),
                    t.entry_bb_position.map_or("n/a".to_string(), |v| format!("{v:.2}")),
                    t.exit_reason.as_deref().unwrap_or("?"),
                    outcome,
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        let trigger_reasons: String = trades
            .iter()
            .map(|t| {
                format!(
                    "- #{} {} {} -> {}",
                    t.id,
                    t.signal_level,
                    t.signal_score,
                    t.llm_reasoning.chars().take(220).collect::<String>()
                )
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
             ## Condition Snapshots (what worked/failed)\n\
             | Trade | Dir | Play | PnL | Level | Score | RSI | CVD | OB | ATR% | BB Pos | Exit | Outcome |\n\
             |------|-----|------|-----|-------|-------|-----|-----|----|------|--------|------|---------|\n\
             {condition_table}\n\n\
             ## Trigger Reasons (LLM response excerpts)\n\
             {trigger_reasons}\n\n\
             ## Patterns Detected\n\
             {patterns}\n\n\
             ## Memory Updates\n\
             {memory_rules}\n\n\
             ## Param adjustments (from LLM)\n\
             Current params: {current_params}\n\
             Suggested:\n\
             {param_section}\n\n\
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
            condition_table = condition_table,
            trigger_reasons = trigger_reasons,
            patterns = patterns,
            memory_rules = memory_rules,
            current_params = current_signal_params,
            param_section = param_section,
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
            param_adjustments: vec![],
        }
    })
}

/// Apply report-suggested signal param adjustments to config and persist to tuned_signals.
pub fn apply_param_adjustments(config: &mut crate::config::Config, adjustments: &[ParamAdjustment]) {
    for adj in adjustments {
        let name = adj.param.as_str();
        let applied = if name == "strong_threshold" {
            adj.value.as_i64().map(|n| {
                config.signals.strong_threshold = n as i32;
                true
            })
        } else if name == "lean_threshold" {
            adj.value.as_i64().map(|n| {
                config.signals.lean_threshold = n as i32;
                true
            })
        } else if name == "min_atr_pct" {
            adj.value.as_f64().map(|v| {
                config.signals.min_atr_pct = v.max(0.0);
                true
            })
        } else if name == "rsi_oversold" {
            adj.value.as_f64().map(|v| {
                config.signals.rsi_oversold = v;
                true
            })
        } else if name == "rsi_overbought" {
            adj.value.as_f64().map(|v| {
                config.signals.rsi_overbought = v;
                true
            })
        } else {
            None
        };
        if applied.unwrap_or(false) {
            info!(param = name, "Applied report param adjustment");
        }
    }
    // Enforce invariants
    if config.signals.strong_threshold < config.signals.lean_threshold {
        config.signals.strong_threshold = config.signals.lean_threshold;
    }
    if config.signals.rsi_oversold > config.signals.rsi_overbought {
        config.signals.rsi_oversold = config.signals.rsi_overbought;
    }
}

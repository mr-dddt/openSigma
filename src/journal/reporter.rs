use std::collections::HashMap;

use anyhow::{Context, Result};
use serde::Deserialize;
use tracing::{info, warn};
use uuid::Uuid;

use crate::agent::llm_client::LlmClient;
use crate::config::Config;
use crate::journal::memory::MemoryManager;
use crate::types::{PlayType, TuneAdjustment, TradeRecord};

const REPORT_SYSTEM_PROMPT: &str = r#"You are openSigma's trade analyst. Given a batch of recent trades and performance metrics, identify patterns and suggest improvements.

Respond with ONLY a valid JSON object:
{
  "patterns": ["pattern 1", "pattern 2"],
  "memory_rules": ["rule: condition (level, score) | outcome (win/loss) | tune: param=val if needed"],
  "summary": "1-2 sentence summary",
  "param_adjustments": [{"param":"strong_threshold","old_value":5,"new_value":6}]
}

Rules:
- memory_rules: use DETAILED indicators, not just level. Format: "rsi=X cvd=Y ob=Z atr%=W bb_pos=V | dir level score | outcome | tune: param=val"
  Example: "rsi=32 cvd=-120 ob=0.85 atr%=0.15 bb_pos=-0.6 | LONG STRONG net=5 | 3/5 wins | tune: strong_threshold=6"
  Always include rsi, cvd, ob, atr%, bb_pos when available — these enable precise learning
- param_adjustments: 0-3 max. Only suggest when data clearly supports it. Tunable: strong_threshold, lean_threshold, ema_cross_weight, cvd_weight, rsi_weight, ob_weight, stoch_rsi_weight, rsi_oversold, rsi_overbought, min_atr_pct
- If a signal level/direction fails often, suggest raising its threshold or lowering weight
- If a signal works well, suggest lowering threshold to catch more
- Keep summary concise"#;

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
    pub param_adjustments: Vec<TuneAdjustment>,
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
    param_adjustments: Vec<TuneAdjustment>,
}

fn default_summary() -> String {
    "No analysis available.".to_string()
}

impl Reporter {
    pub fn new(api_key: &str, model: &str) -> Result<Self> {
        let llm_client = LlmClient::new(
            api_key.to_string(),
            model.to_string(),
            10_000, // longer timeout for report analysis
        )?;
        Ok(Self {
            llm_client,
            report_dir: "data/reports".to_string(),
        })
    }

    /// Generate a report from a batch of trades, update memory, return result.
    pub async fn generate(
        &self,
        trades: &[TradeRecord],
        memory: &MemoryManager,
        config: &Config,
    ) -> Result<ReportResult> {
        if trades.is_empty() {
            anyhow::bail!("No trades to report on");
        }

        let metrics = compute_metrics(trades);
        let report_num = self.next_report_number();

        // Build LLM context
        let context = self.build_context(&metrics, trades, memory, config);

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
        let report_content = self.format_report(report_num, &metrics, &analysis);
        std::fs::write(&report_path, &report_content)
            .with_context(|| format!("Failed to write report to {report_path}"))?;

        // Append memory rules (fallback to summary if LLM returns none)
        let memory_additions: Vec<String> = if analysis.memory_rules.is_empty() {
            vec![analysis.summary.clone()]
        } else {
            analysis.memory_rules.clone()
        };
        let param_str = if analysis.param_adjustments.is_empty() {
            String::new()
        } else {
            format!(
                "\nTune: {}",
                analysis
                    .param_adjustments
                    .iter()
                    .map(|a| format!("{}={}", a.param, a.new_value))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        if !memory_additions.is_empty() {
            let memory_block = format!(
                "\n## Report #{} — {}\n{}{}",
                report_num,
                chrono::Utc::now().format("%Y-%m-%d %H:%M UTC"),
                memory_additions
                    .iter()
                    .map(|r| format!("- {r}"))
                    .collect::<Vec<_>>()
                    .join("\n"),
                param_str,
            );
            memory.append(&memory_block)?;
        }

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
                    .filter(|e| {
                        e.file_name()
                            .to_string_lossy()
                            .starts_with("report_")
                    })
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
        config: &Config,
    ) -> String {
        let trades_detail: String = trades
            .iter()
            .map(|t| {
                let ind = format!(
                    "rsi={} cvd={} ob={} atr%={} bb_pos={}",
                    t.entry_rsi.map(|v| format!("{v:.0}")).unwrap_or_else(|| "N/A".to_string()),
                    t.entry_cvd.map(|v| format!("{v:.1}")).unwrap_or_else(|| "N/A".to_string()),
                    t.entry_ob.map(|v| format!("{v:.2}")).unwrap_or_else(|| "N/A".to_string()),
                    t.entry_atr_pct.map(|v| format!("{v:.3}")).unwrap_or_else(|| "N/A".to_string()),
                    t.entry_bb_position.map(|v| format!("{v:.2}")).unwrap_or_else(|| "N/A".to_string()),
                );
                format!(
                    "  #{} {} {} pnl=${:.2} level={} score={} exit={} dur={}s | {}",
                    t.id,
                    t.direction,
                    t.play_type,
                    t.pnl_usd.unwrap_or(0.0),
                    t.signal_level,
                    t.signal_score,
                    t.exit_reason.as_deref().unwrap_or("?"),
                    t.duration_secs.unwrap_or(0),
                    ind,
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

        let signals_json = serde_json::to_string_pretty(&config.signals).unwrap_or_default();
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
             Current signal params (tunable):\n{}\n\n\
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
            signals_json,
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
            param_adjustments: vec![],
        }
    })
}

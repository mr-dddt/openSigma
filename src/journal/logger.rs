use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};

use anyhow::{Context, Result};
use tracing::info;

use crate::types::TradeRecord;

/// JSONL trade journal — appends every trade as structured JSON.
pub struct TradeLogger {
    path: String,
}

impl TradeLogger {
    pub fn new(path: &str) -> Self {
        info!(path = path, "TradeLogger initialized");
        Self {
            path: path.to_string(),
        }
    }

    /// Append a trade record to the JSONL file.
    pub fn log_entry(&self, record: &TradeRecord) -> Result<()> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .with_context(|| format!("Failed to open journal: {}", self.path))?;

        let line = serde_json::to_string(record)
            .context("Failed to serialize trade record")?;
        writeln!(file, "{}", line)?;

        info!(id = %record.id, pnl = record.pnl_usd, "Trade logged");
        Ok(())
    }

    /// Read the most recent N trade records.
    pub fn read_recent(&self, n: usize) -> Result<Vec<TradeRecord>> {
        let file = match std::fs::File::open(&self.path) {
            Ok(f) => f,
            Err(_) => return Ok(vec![]),
        };

        let reader = BufReader::new(file);
        let all_lines: Vec<String> = reader.lines().filter_map(|l| l.ok()).collect();

        let records: Vec<TradeRecord> = all_lines
            .iter()
            .rev()
            .take(n)
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect();

        Ok(records)
    }

    /// Count total trades logged.
    pub fn trade_count(&self) -> usize {
        match std::fs::File::open(&self.path) {
            Ok(f) => BufReader::new(f).lines().count(),
            Err(_) => 0,
        }
    }
}

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

    /// Read the most recent N *closed* trade records (ts_close is Some).
    /// Open entries (journaled on position open) are filtered out to avoid
    /// polluting metrics with phantom $0 PnL "wins".
    pub fn read_recent(&self, n: usize) -> Result<Vec<TradeRecord>> {
        let file = match std::fs::File::open(&self.path) {
            Ok(f) => f,
            Err(_) => return Ok(vec![]),
        };

        let reader = BufReader::new(file);
        let all_lines: Vec<String> = reader.lines().map_while(Result::ok).collect();

        let mut records: Vec<TradeRecord> = all_lines
            .iter()
            .rev()
            .filter_map(|line| serde_json::from_str::<TradeRecord>(line).ok())
            .filter(|r| r.ts_close.is_some())
            .take(n)
            .collect();
        records.reverse();

        Ok(records)
    }

    /// Read all closed trade records (for startup stats restoration).
    pub fn read_all_closed(&self) -> Result<Vec<TradeRecord>> {
        let file = match std::fs::File::open(&self.path) {
            Ok(f) => f,
            Err(_) => return Ok(vec![]),
        };

        let reader = BufReader::new(file);
        let records: Vec<TradeRecord> = reader
            .lines()
            .map_while(Result::ok)
            .filter_map(|line| serde_json::from_str::<TradeRecord>(&line).ok())
            .filter(|r| r.ts_close.is_some())
            .collect();

        Ok(records)
    }

    /// Count total trades logged.
    #[allow(dead_code)]
    pub fn trade_count(&self) -> usize {
        match std::fs::File::open(&self.path) {
            Ok(f) => BufReader::new(f).lines().count(),
            Err(_) => 0,
        }
    }
}

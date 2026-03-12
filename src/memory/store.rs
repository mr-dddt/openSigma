use anyhow::Result;
use chrono::Utc;
use rusqlite::Connection;

use tracing::info;

use crate::types::*;

/// Persistent memory: append-only `memory.md` + SQLite `trades.db`.
pub struct MemoryStore {
    memory_path: String,
    db: Connection,
}

impl MemoryStore {
    pub fn new(memory_dir: &str) -> Result<Self> {
        let memory_path = format!("{}/memory.md", memory_dir);
        let db_path = format!("{}/trades.db", memory_dir);

        let db = Connection::open(&db_path)?;
        db.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS trades (
                id TEXT PRIMARY KEY,
                proposer TEXT NOT NULL,
                symbol TEXT NOT NULL,
                direction TEXT NOT NULL,
                size_usd REAL NOT NULL,
                leverage REAL NOT NULL,
                entry_price REAL NOT NULL,
                stop_loss REAL NOT NULL,
                take_profit REAL NOT NULL,
                exit_price REAL,
                pnl REAL,
                rationale TEXT NOT NULL,
                verdict TEXT NOT NULL,
                verdict_reason TEXT NOT NULL,
                opened_at TEXT NOT NULL,
                closed_at TEXT,
                lesson TEXT
            );
            ",
        )?;

        info!(db = %db_path, "MemoryStore initialized");

        Ok(Self { memory_path, db })
    }

    /// Record a trade decision (open).
    pub fn record_open(&self, decision: &TradeDecision) -> Result<()> {
        self.db.execute(
            "INSERT INTO trades (id, proposer, symbol, direction, size_usd, leverage,
             entry_price, stop_loss, take_profit, rationale, verdict, verdict_reason, opened_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            rusqlite::params![
                decision.proposal.id.to_string(),
                decision.proposal.proposer.to_string(),
                decision.proposal.symbol.to_string(),
                format!("{:?}", decision.proposal.direction),
                decision.adjusted_size.unwrap_or(decision.proposal.size_usd),
                decision.adjusted_leverage.unwrap_or(decision.proposal.leverage),
                decision.proposal.entry_price,
                decision.proposal.stop_loss,
                decision.proposal.take_profit,
                decision.proposal.rationale,
                format!("{:?}", decision.verdict),
                decision.reason,
                decision.timestamp.to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    /// Record trade close and append lesson to memory.md.
    pub fn record_close(
        &self,
        trade_id: &str,
        exit_price: f64,
        pnl: f64,
        lesson: &str,
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();

        self.db.execute(
            "UPDATE trades SET exit_price = ?1, pnl = ?2, closed_at = ?3, lesson = ?4
             WHERE id = ?5",
            rusqlite::params![exit_price, pnl, now, lesson, trade_id],
        )?;

        // Append to memory.md
        self.append_memory(trade_id, pnl, lesson)?;

        Ok(())
    }

    fn append_memory(&self, trade_id: &str, pnl: f64, lesson: &str) -> Result<()> {
        use std::fs::OpenOptions;
        use std::io::Write;

        let mut file = OpenOptions::new()
            .append(true)
            .open(&self.memory_path)?;

        let pnl_str = if pnl >= 0.0 {
            format!("+{:.2}%", pnl)
        } else {
            format!("{:.2}%", pnl)
        };

        writeln!(
            file,
            "\n## [{}] Trade {} — CLOSED {}\n- Lesson: {}\n",
            Utc::now().format("%Y-%m-%d %H:%M"),
            trade_id,
            pnl_str,
            lesson,
        )?;

        Ok(())
    }

    /// Read the full memory.md contents for agent consumption.
    pub fn read_memory(&self) -> Result<String> {
        let content = std::fs::read_to_string(&self.memory_path)?;
        Ok(content)
    }

    /// Get all open trades (no closed_at).
    pub fn open_trades(&self) -> Result<Vec<String>> {
        let mut stmt = self.db.prepare("SELECT id FROM trades WHERE closed_at IS NULL")?;
        let ids: Vec<String> = stmt
            .query_map([], |row| row.get(0))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(ids)
    }

    /// Get the SQLite connection for advanced queries.
    pub fn db(&self) -> &Connection {
        &self.db
    }
}

use anyhow::Result;
use tracing::info;

/// Memory system — reads/writes memory.md for LLM system prompt context.
pub struct MemoryManager {
    path: String,
}

impl MemoryManager {
    pub fn new(path: &str) -> Self {
        info!(path = path, "MemoryManager initialized");
        Self {
            path: path.to_string(),
        }
    }

    /// Read the current memory file content (injected into LLM system prompt).
    pub fn recent_summary(&self) -> String {
        std::fs::read_to_string(&self.path).unwrap_or_else(|_| {
            "No memory yet — first trading session.".to_string()
        })
    }

    /// Update memory with new content (e.g., after tuning or 20-trade report).
    pub fn update(&self, content: &str) -> Result<()> {
        std::fs::write(&self.path, content)?;
        info!("Memory updated");
        Ok(())
    }

    /// Append a line to the memory file.
    pub fn append(&self, line: &str) -> Result<()> {
        use std::fs::OpenOptions;
        use std::io::Write;

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        writeln!(file, "{}", line)?;
        Ok(())
    }
}

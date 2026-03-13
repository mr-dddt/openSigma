use anyhow::Result;
use tracing::info;

const MAX_REPORT_SECTIONS: usize = 5;
const SECTION_MARKER: &str = "\n## Report #";

/// Memory system — reads/writes memory.md for LLM system prompt context.
/// Automatically prunes old report sections to keep memory focused and
/// token cost bounded. Only the most recent MAX_REPORT_SECTIONS are kept;
/// older lessons should be coded into config/signals by human review.
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
    #[allow(dead_code)]
    pub fn update(&self, content: &str) -> Result<()> {
        std::fs::write(&self.path, content)?;
        info!("Memory updated");
        Ok(())
    }

    /// Append a report section to memory, then prune to keep only the most
    /// recent MAX_REPORT_SECTIONS report blocks. Non-report content at the
    /// top of the file (e.g. "## Validated Rules", "## Soft Patterns") is
    /// always preserved.
    pub fn append(&self, line: &str) -> Result<()> {
        use std::fs::OpenOptions;
        use std::io::Write;

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        writeln!(file, "{}", line)?;
        drop(file);

        self.prune()?;
        Ok(())
    }

    /// Keep only the most recent MAX_REPORT_SECTIONS report sections.
    /// Splits on "## Report #" markers. The preamble (everything before
    /// the first report section) is always kept.
    fn prune(&self) -> Result<()> {
        let content = match std::fs::read_to_string(&self.path) {
            Ok(c) => c,
            Err(_) => return Ok(()),
        };

        let parts: Vec<&str> = content.split(SECTION_MARKER).collect();
        if parts.len() <= MAX_REPORT_SECTIONS + 1 {
            return Ok(());
        }

        // parts[0] = preamble (validated rules, soft patterns, etc.)
        // parts[1..] = report sections (oldest first)
        let preamble = parts[0];
        let report_sections = &parts[1..];
        let keep_from = report_sections.len().saturating_sub(MAX_REPORT_SECTIONS);
        let kept: Vec<&str> = report_sections[keep_from..].to_vec();

        let mut pruned = preamble.to_string();
        for section in kept {
            pruned.push_str(SECTION_MARKER);
            pruned.push_str(section);
        }

        std::fs::write(&self.path, pruned)?;
        info!(
            kept = MAX_REPORT_SECTIONS,
            pruned = keep_from,
            "Memory pruned to most recent report sections"
        );
        Ok(())
    }
}

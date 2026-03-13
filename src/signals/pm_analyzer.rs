use crate::types::*;

/// Polymarket odds analyzer — detects divergence and spread opportunities.
/// Phase 1: basic divergence detection. Phase 2: full arbitrage logic.
#[allow(dead_code)] // Phase 2 stub — will be wired when PM SDK is integrated
pub struct PmAnalyzer {
    latest_odds: Option<PmOdds>,
}

#[allow(dead_code)]
impl PmAnalyzer {
    pub fn new() -> Self {
        Self { latest_odds: None }
    }

    pub fn update(&mut self, odds: PmOdds) {
        self.latest_odds = Some(odds);
    }

    /// Returns the current "Up" price (0.0–1.0) if available.
    pub fn up_price(&self) -> Option<f64> {
        self.latest_odds.as_ref().map(|o| o.up_price)
    }

    /// Detect if Up + Down < 1.0 (spread capture opportunity).
    pub fn has_spread(&self) -> Option<f64> {
        self.latest_odds.as_ref().and_then(|o| {
            let total = o.up_price + o.down_price;
            if total < 0.98 {
                Some(1.0 - total)
            } else {
                None
            }
        })
    }
}

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::config::ExecutionConfig;
use crate::types::{ActiveTrade, Direction};

fn pnl_pct(trade: &ActiveTrade, current_price: f64) -> f64 {
    match trade.direction {
        Direction::Long => (current_price - trade.entry_price) / trade.entry_price * 100.0,
        Direction::Short => (trade.entry_price - current_price) / trade.entry_price * 100.0,
    }
}

fn round_trip_cost_pct(exec_cfg: &ExecutionConfig) -> f64 {
    // bps -> %
    (exec_cfg.fee_bps_per_side.max(0.0) * 2.0) / 100.0 + exec_cfg.sell_trigger_extra_buffer_pct.max(0.0)
}

/// Proactive exit policy focused on harvesting early edge and preventing
/// winner-to-loser giveback in ultra-short BTC scalps.
pub fn evaluate_proactive_exits(
    trades: &[ActiveTrade],
    current_price: f64,
    now: DateTime<Utc>,
    exec_cfg: &ExecutionConfig,
    peak_pnl_pct: &mut HashMap<Uuid, f64>,
) -> Vec<(Uuid, String)> {
    if !exec_cfg.enable_proactive_exit || current_price <= 0.0 {
        return Vec::new();
    }

    let mut exits = Vec::new();
    let cost_pct = round_trip_cost_pct(exec_cfg);
    for t in trades {
        let gross_p = pnl_pct(t, current_price);
        let net_p = gross_p - cost_pct;
        let peak = peak_pnl_pct.entry(t.id).or_insert(net_p.max(0.0));
        if net_p > *peak {
            *peak = net_p;
        }

        let elapsed = (now - t.opened_at).num_seconds().max(0) as u64;

        // 1) Early alpha harvest: if the setup works quickly, close quickly.
        if elapsed <= exec_cfg.fast_take_window_secs && net_p >= exec_cfg.fast_take_trigger_pct {
            exits.push((t.id, "fast_take".to_string()));
            continue;
        }

        // 2) Profit lock once trade had enough positive excursion.
        if *peak >= exec_cfg.profit_lock_trigger_pct && net_p <= exec_cfg.profit_lock_floor_pct {
            exits.push((t.id, "profit_lock".to_string()));
            continue;
        }

        // 3) Peak giveback: avoid letting winners round-trip.
        if *peak >= exec_cfg.profit_lock_trigger_pct {
            let giveback_ratio = exec_cfg.peak_giveback_ratio.clamp(0.0, 0.95);
            let giveback_floor = *peak * (1.0 - giveback_ratio);
            if net_p > 0.0 && net_p <= giveback_floor {
                exits.push((t.id, "giveback_exit".to_string()));
                continue;
            }
        }

        // 4) Time decay: if trade goes stale with little/negative edge, exit.
        if elapsed >= exec_cfg.stale_after_secs && net_p <= exec_cfg.stale_min_pnl_pct {
            exits.push((t.id, "stale_exit".to_string()));
            continue;
        }
    }

    exits
}

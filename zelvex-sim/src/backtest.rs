use zelvex_types::OpportunityRecord;

/// A single opportunity where the recomputed decision differs from the original.
#[derive(Debug, Clone)]
pub struct BacktestMismatch {
    pub opportunity_id: i64,
    pub original_decision: String,
    pub recomputed_decision: String,
    pub original_no_go_reason: Option<String>,
    pub recomputed_no_go_reason: Option<String>,
    pub estimated_profit_usd: f64,
    pub gas_estimate_usd: f64,
    pub net_profit_usd: f64,
}

#[derive(Debug, Clone)]
pub struct BacktestReport {
    pub total: usize,
    pub matched: usize,
    pub mismatched: usize,
    /// 0.0–100.0
    pub match_rate_pct: f64,
    /// Sum of `estimated_profit_usd` for all rows where original decision was `"go"`.
    pub total_estimated_profit_go_usd: f64,
    /// Sum of `gas_estimate_usd` for all rows where original decision was `"go"`.
    pub total_gas_cost_go_usd: f64,
    pub mismatches: Vec<BacktestMismatch>,
}

/// Re-evaluate each stored opportunity against `min_profit_usd` and compare
/// the result to the recorded decision.
///
/// The simulation formula is identical to [`crate::evaluate`]:
/// `net_profit = estimated_profit_usd - gas_estimate_usd`
/// Decision: `"go"` if `net_profit > min_profit_usd`, else `"no-go"`.
///
/// This function is pure — no I/O, no async.
pub fn run_backtest(rows: &[OpportunityRecord], min_profit_usd: f64) -> BacktestReport {
    let mut matched = 0usize;
    let mut mismatched = 0usize;
    let mut total_estimated_profit_go_usd = 0.0f64;
    let mut total_gas_cost_go_usd = 0.0f64;
    let mut mismatches = Vec::new();

    for row in rows {
        let net_profit = row.estimated_profit_usd - row.gas_estimate_usd;
        let (recomputed_decision, recomputed_no_go_reason) = if net_profit > min_profit_usd {
            ("go".to_string(), None)
        } else {
            (
                "no-go".to_string(),
                Some(format!(
                    "net_profit_usd {:.4} <= min_profit_usd {:.4}",
                    net_profit, min_profit_usd
                )),
            )
        };

        if row.original_decision == "go" {
            total_estimated_profit_go_usd += row.estimated_profit_usd;
            total_gas_cost_go_usd += row.gas_estimate_usd;
        }

        if row.original_decision == recomputed_decision {
            matched += 1;
        } else {
            mismatched += 1;
            mismatches.push(BacktestMismatch {
                opportunity_id: row.id,
                original_decision: row.original_decision.clone(),
                recomputed_decision,
                original_no_go_reason: row.original_no_go_reason.clone(),
                recomputed_no_go_reason,
                estimated_profit_usd: row.estimated_profit_usd,
                gas_estimate_usd: row.gas_estimate_usd,
                net_profit_usd: net_profit,
            });
        }
    }

    let total = rows.len();
    let match_rate_pct = if total == 0 {
        100.0
    } else {
        (matched as f64 / total as f64) * 100.0
    };

    BacktestReport {
        total,
        matched,
        mismatched,
        match_rate_pct,
        total_estimated_profit_go_usd,
        total_gas_cost_go_usd,
        mismatches,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zelvex_types::OpportunityRecord;

    fn make_row(id: i64, profit: f64, gas: f64, decision: &str) -> OpportunityRecord {
        OpportunityRecord {
            id,
            estimated_profit_usd: profit,
            gas_estimate_usd: gas,
            spread_bps: 10,
            original_decision: decision.to_string(),
            original_no_go_reason: if decision == "no-go" {
                Some("net_profit_usd 0.5000 <= min_profit_usd 5.0000".to_string())
            } else {
                None
            },
            timestamp: 1_700_000_000,
        }
    }

    #[test]
    fn empty_input_returns_zero_totals_with_full_match_rate() {
        let report = run_backtest(&[], 5.0);
        assert_eq!(report.total, 0);
        assert_eq!(report.matched, 0);
        assert_eq!(report.mismatched, 0);
        assert_eq!(report.match_rate_pct, 100.0);
        assert!(report.mismatches.is_empty());
    }

    #[test]
    fn all_go_decisions_match() {
        // profit 20, gas 2 → net 18 > 5 → recomputed "go" matches original "go"
        let rows = vec![
            make_row(1, 20.0, 2.0, "go"),
            make_row(2, 15.0, 3.0, "go"),
        ];
        let report = run_backtest(&rows, 5.0);
        assert_eq!(report.total, 2);
        assert_eq!(report.matched, 2);
        assert_eq!(report.mismatched, 0);
        assert_eq!(report.match_rate_pct, 100.0);
        assert!(report.mismatches.is_empty());
    }

    #[test]
    fn all_no_go_decisions_match() {
        // profit 3, gas 2 → net 1 <= 5 → recomputed "no-go" matches original "no-go"
        let rows = vec![
            make_row(1, 3.0, 2.0, "no-go"),
            make_row(2, 4.0, 2.0, "no-go"),
        ];
        let report = run_backtest(&rows, 5.0);
        assert_eq!(report.total, 2);
        assert_eq!(report.matched, 2);
        assert_eq!(report.mismatched, 0);
        assert_eq!(report.match_rate_pct, 100.0);
    }

    #[test]
    fn raised_threshold_causes_mismatch() {
        // Originally logged as "go" at min_profit=5. Re-run at min_profit=15 → mismatch.
        // profit 12, gas 2 → net 10. At threshold 15: net 10 <= 15 → "no-go".
        let rows = vec![make_row(1, 12.0, 2.0, "go")];
        let report = run_backtest(&rows, 15.0);
        assert_eq!(report.total, 1);
        assert_eq!(report.matched, 0);
        assert_eq!(report.mismatched, 1);
        assert_eq!(report.match_rate_pct, 0.0);
        assert_eq!(report.mismatches[0].opportunity_id, 1);
        assert_eq!(report.mismatches[0].original_decision, "go");
        assert_eq!(report.mismatches[0].recomputed_decision, "no-go");
        assert!((report.mismatches[0].net_profit_usd - 10.0).abs() < 1e-9);
    }

    #[test]
    fn lowered_threshold_causes_mismatch() {
        // Originally logged as "no-go" at min_profit=5. Re-run at min_profit=1 → mismatch.
        // profit 4, gas 2 → net 2 > 1 → "go".
        let rows = vec![make_row(1, 4.0, 2.0, "no-go")];
        let report = run_backtest(&rows, 1.0);
        assert_eq!(report.total, 1);
        assert_eq!(report.mismatched, 1);
        assert_eq!(report.mismatches[0].recomputed_decision, "go");
    }

    #[test]
    fn mixed_decisions_correct_stats() {
        let rows = vec![
            make_row(1, 20.0, 2.0, "go"),   // net 18 > 5 → go, matches
            make_row(2, 3.0, 2.0, "no-go"), // net 1 <= 5 → no-go, matches
            make_row(3, 12.0, 2.0, "go"),   // net 10 > 5 → go, matches
            make_row(4, 4.5, 2.0, "go"),    // net 2.5 <= 5 → no-go, MISMATCH
        ];
        let report = run_backtest(&rows, 5.0);
        assert_eq!(report.total, 4);
        assert_eq!(report.matched, 3);
        assert_eq!(report.mismatched, 1);
        assert_eq!(report.match_rate_pct, 75.0);
        assert_eq!(report.mismatches[0].opportunity_id, 4);
    }

    #[test]
    fn profit_sums_only_count_original_go_rows() {
        let rows = vec![
            make_row(1, 20.0, 2.0, "go"),   // included in sums
            make_row(2, 3.0, 2.0, "no-go"), // excluded from sums
            make_row(3, 10.0, 1.5, "go"),   // included in sums
        ];
        let report = run_backtest(&rows, 5.0);
        assert!((report.total_estimated_profit_go_usd - 30.0).abs() < 1e-9);
        assert!((report.total_gas_cost_go_usd - 3.5).abs() < 1e-9);
    }

    #[test]
    fn zero_net_profit_is_no_go() {
        // profit == gas_estimate → net 0 which is not > min_profit 5
        let rows = vec![make_row(1, 5.0, 5.0, "no-go")];
        let report = run_backtest(&rows, 5.0);
        assert_eq!(report.matched, 1);
    }
}

use runner_api::DatabaseRunner;
use workload_generator::Operation;

#[derive(Debug)]
pub struct ValidationReport {
    pub total_ops:  usize,
    pub mismatches: usize,
    pub error_rate: f64,
}

/// Run the same operations against two runners and report where one errors
/// while the other doesn't (or where latency differs by more than 10x).
pub fn cross_validate(
    ground_truth: &mut dyn DatabaseRunner,
    candidate:    &mut dyn DatabaseRunner,
    users:        &[data_generator::User],
    ops:          &[Operation],
) -> ValidationReport {
    let _ = ground_truth.load_data(users);
    let _ = candidate.load_data(users);

    let mut mismatches = 0usize;

    for op in ops {
        let gt_result = ground_truth.execute(op);
        let ca_result = candidate.execute(op);

        match (&gt_result, &ca_result) {
            (Ok(gt), Ok(ca)) => {
                // Flag if latency differs by more than 10x
                let gt_us = gt.latency.as_micros().max(1);
                let ca_us = ca.latency.as_micros().max(1);
                if gt_us * 10 < ca_us || ca_us * 10 < gt_us {
                    log::warn!(
                        "Latency divergence: gt={}µs candidate={}µs",
                        gt_us, ca_us
                    );
                    mismatches += 1;
                }
            }
            (Ok(_), Err(e)) => {
                log::warn!("Candidate errored but ground truth succeeded: {}", e);
                mismatches += 1;
            }
            (Err(e), Ok(_)) => {
                log::warn!("Ground truth errored but candidate succeeded: {}", e);
                mismatches += 1;
            }
            (Err(_), Err(_)) => {
                // Both errored — not a mismatch
            }
        }
    }

    let total_ops = ops.len();
    let error_rate = if total_ops == 0 {
        0.0
    } else {
        mismatches as f64 / total_ops as f64
    };

    ValidationReport { total_ops, mismatches, error_rate }
}

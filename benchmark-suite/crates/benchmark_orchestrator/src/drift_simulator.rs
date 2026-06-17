use runner_api::DatabaseRunner;
use workload_generator::{Phase, WorkloadConfig, WorkloadGenerator};

/// Returns the sequence of phases for drift simulation.
/// Using a function instead of a const to avoid non-Copy enum in static context.
pub fn drift_schedule() -> Vec<Phase> {
    vec![
        Phase::IAM,
        Phase::Compliance,
        Phase::Tenant,
        Phase::IAM,
        Phase::Security,
        Phase::IAM,
    ]
}

#[derive(Debug)]
pub struct DriftResult {
    pub phase_index:            usize,
    pub phase:                  Phase,
    pub metrics:                metrics::Metrics,
    pub adaptation_latency_ops: Option<usize>,
}

pub fn run_drift_simulation(
    runner:          &mut dyn DatabaseRunner,
    users:           &[data_generator::User],
    ops_per_phase:   usize,
    workload_config: &WorkloadConfig,
) -> Vec<DriftResult> {
    if let Err(e) = runner.load_data(users) {
        log::error!("load_data failed: {}", e);
        return vec![];
    }

    let schedule = drift_schedule();
    let mut results = Vec::new();

    for (phase_index, phase) in schedule.into_iter().enumerate() {
        log::info!("DriftSim: phase_index={} phase={:?}", phase_index, phase);
        runner.reset_metrics();

        let mut wl = WorkloadGenerator::new(workload_config.clone(), phase.clone(), users.len());
        let mut adaptation_latency_ops: Option<usize> = None;
        let snapshot_interval = 100_000.min(ops_per_phase).max(1);

        for i in 0..ops_per_phase {
            let op = wl.next_operation();
            if let Err(e) = runner.execute(&op) {
                log::warn!("execute error: {}", e);
            }

            if i > 0 && i % snapshot_interval == 0 && adaptation_latency_ops.is_none() {
                let m = runner.collect_metrics();
                if m.factor_utilization >= 0.9 {
                    adaptation_latency_ops = Some(i);
                }
            }
        }

        let metrics = runner.collect_metrics();
        results.push(DriftResult {
            phase_index,
            phase,
            metrics,
            adaptation_latency_ops,
        });
    }

    results
}

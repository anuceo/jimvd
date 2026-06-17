use runner_api::DatabaseRunner;
use workload_generator::{Phase, WorkloadConfig, WorkloadGenerator};

#[derive(Debug)]
pub struct EvolutionSnapshot {
    pub operations: usize,
    pub metrics:    metrics::Metrics,
}

pub fn run_long_term_evolution(
    runner:               &mut dyn DatabaseRunner,
    users:                &[data_generator::User],
    total_ops:            usize,
    snapshot_interval:    usize,
    phase_shift_interval: usize,
    read_ratio:           f64,
    write_ratio:          f64,
) -> Vec<EvolutionSnapshot> {
    if let Err(e) = runner.load_data(users) {
        log::error!("load_data failed: {}", e);
        return vec![];
    }

    let phases = [Phase::IAM, Phase::Compliance, Phase::Tenant, Phase::Security];
    let mut phase_idx = 0usize;

    let wl_config = WorkloadConfig {
        read_ratio,
        write_ratio,
        join_ratio: 0.0,
        total_operations: total_ops,
    };

    let mut wl = WorkloadGenerator::new(
        wl_config.clone(),
        phases[phase_idx].clone(),
        users.len(),
    );

    let mut snapshots = Vec::new();

    for i in 0..total_ops {
        if phase_shift_interval > 0 && i > 0 && i % phase_shift_interval == 0 {
            phase_idx = (phase_idx + 1) % phases.len();
            wl = WorkloadGenerator::new(
                wl_config.clone(),
                phases[phase_idx].clone(),
                users.len(),
            );
        }

        let op = wl.next_operation();
        if let Err(e) = runner.execute(&op) {
            log::warn!("execute error: {}", e);
        }

        if snapshot_interval > 0 && i > 0 && i % snapshot_interval == 0 {
            let metrics = runner.collect_metrics();
            snapshots.push(EvolutionSnapshot { operations: i, metrics });
        }
    }

    snapshots
}

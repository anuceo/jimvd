use data_generator::CorrelationConfig;
use runner_api::DatabaseRunner;
use workload_generator::{Phase, WorkloadConfig, WorkloadGenerator};

pub const SCALES: &[usize] = &[1_000, 10_000, 100_000, 1_000_000, 10_000_000];

#[derive(Debug)]
pub struct ScalingResult {
    pub scale:       usize,
    pub runner_name: String,
    pub metrics:     metrics::Metrics,
}

pub fn run_scaling_wall(
    runner:          &mut dyn DatabaseRunner,
    config:          &CorrelationConfig,
    workload_config: &WorkloadConfig,
    phase:           Phase,
    max_scale:       usize,
) -> Vec<ScalingResult> {
    let mut results = Vec::new();

    for &scale in SCALES {
        if scale > max_scale {
            break;
        }
        log::info!("ScalingWall: scale={}", scale);

        let users = data_generator::generate_users(scale, config);
        runner.reset_metrics();

        if let Err(e) = runner.load_data(&users) {
            log::error!("load_data failed at scale {}: {}", scale, e);
            continue;
        }

        let mut wl = WorkloadGenerator::new(
            workload_config.clone(),
            phase.clone(),
            scale,
        );

        for _ in 0..workload_config.total_operations {
            let op = wl.next_operation();
            if let Err(e) = runner.execute(&op) {
                log::warn!("execute error: {}", e);
            }
        }

        let metrics = runner.collect_metrics();
        results.push(ScalingResult {
            scale,
            runner_name: runner.name().to_string(),
            metrics,
        });
    }

    results
}

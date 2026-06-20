use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
use runner_api::DatabaseRunner;
use workload_generator::{Phase, WorkloadConfig, WorkloadGenerator};

pub fn main_event_schedule() -> Vec<Phase> {
    vec![
        Phase::IAM,
        Phase::Compliance,
        Phase::Tenant,
        Phase::IAM,
        Phase::Security,
        Phase::IAM,
    ]
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PhaseSnapshot {
    pub phase:          String,
    pub phase_index:    usize,
    pub ops_into_phase: usize,
    pub total_ops:      usize,
    pub metrics:        metrics::Metrics,
}

#[derive(Debug, serde::Serialize)]
pub struct AdaptationRecord {
    pub phase:       String,
    pub phase_index: usize,
    pub latency_ops: Option<usize>,
}

#[derive(Debug, serde::Serialize)]
pub struct MainEventResult {
    pub runner_name:          String,
    pub snapshots:            Vec<PhaseSnapshot>,
    pub adaptation_latencies: Vec<AdaptationRecord>,
    pub total_elapsed_secs:   f64,
    /// True if the run was cut short by a stop signal.
    pub interrupted:          bool,
}

pub fn run_main_event(
    runner:               &mut dyn DatabaseRunner,
    users:                &[data_generator::User],
    ops_per_phase:        usize,
    snapshot_interval:    usize,
    adaptation_threshold: f64,
    stop:                 Arc<AtomicBool>,
) -> MainEventResult {
    let runner_name = runner.name().to_string();
    let wall = Instant::now();

    if let Err(e) = runner.load_data(users) {
        log::error!("[{}] load_data failed: {}", runner_name, e);
        return MainEventResult {
            runner_name,
            snapshots: vec![],
            adaptation_latencies: vec![],
            total_elapsed_secs: wall.elapsed().as_secs_f64(),
            interrupted: false,
        };
    }

    let schedule = main_event_schedule();
    let mut snapshots = Vec::new();
    let mut adaptation_latencies = Vec::new();
    let mut total_ops = 0usize;
    let mut interrupted = false;
    let wl_cfg = WorkloadConfig {
        read_ratio:       0.6,
        write_ratio:      0.4,
        join_ratio:       0.0,
        total_operations: ops_per_phase,
        rng_seed:         0,
    };

    'phases: for (phase_index, phase) in schedule.into_iter().enumerate() {
        let phase_label = format!("{:?}", phase);
        log::info!("[{}] phase {}: {:?} ({} ops)", runner_name, phase_index, phase, ops_per_phase);
        runner.reset_metrics();

        let mut wl = WorkloadGenerator::new(wl_cfg.clone(), phase.clone(), users.len());
        let mut adaptation_latency: Option<usize> = None;

        for i in 1..=ops_per_phase {
            if stop.load(Ordering::Relaxed) {
                log::warn!("[{}] stop signal received at phase {} op {}", runner_name, phase_index, i);
                // Flush a snapshot at the interruption point.
                let m = runner.collect_metrics();
                if adaptation_latency.is_none() && m.factor_utilization >= adaptation_threshold {
                    adaptation_latency = Some(i);
                }
                snapshots.push(PhaseSnapshot {
                    phase:          phase_label.clone(),
                    phase_index,
                    ops_into_phase: i,
                    total_ops:      total_ops + i,
                    metrics:        m,
                });
                adaptation_latencies.push(AdaptationRecord {
                    phase:       phase_label,
                    phase_index,
                    latency_ops: adaptation_latency,
                });
                interrupted = true;
                break 'phases;
            }

            let op = wl.next_operation();
            if let Err(e) = runner.execute(&op) {
                log::warn!("[{}] execute error: {}", runner_name, e);
            }

            if i % snapshot_interval == 0 {
                let m = runner.collect_metrics();
                if adaptation_latency.is_none() && m.factor_utilization >= adaptation_threshold {
                    adaptation_latency = Some(i);
                }
                snapshots.push(PhaseSnapshot {
                    phase:          phase_label.clone(),
                    phase_index,
                    ops_into_phase: i,
                    total_ops:      total_ops + i,
                    metrics:        m,
                });
            }
        }

        if interrupted { break; }

        // Final snapshot for the phase even if it doesn't land on the interval.
        let m = runner.collect_metrics();
        if adaptation_latency.is_none() && m.factor_utilization >= adaptation_threshold {
            adaptation_latency = Some(ops_per_phase);
        }
        snapshots.push(PhaseSnapshot {
            phase:          phase_label.clone(),
            phase_index,
            ops_into_phase: ops_per_phase,
            total_ops:      total_ops + ops_per_phase,
            metrics:        m,
        });

        adaptation_latencies.push(AdaptationRecord {
            phase:       phase_label,
            phase_index,
            latency_ops: adaptation_latency,
        });
        total_ops += ops_per_phase;
    }

    MainEventResult {
        runner_name,
        snapshots,
        adaptation_latencies,
        total_elapsed_secs: wall.elapsed().as_secs_f64(),
        interrupted,
    }
}

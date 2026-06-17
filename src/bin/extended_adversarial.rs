use anyhow::Result;
use jimvd::benchmark::BenchmarkRunner;
use jimvd::workload::WorkloadConfig;
use serde::Deserialize;
use std::fs;

#[derive(Debug, Deserialize)]
struct ExtendedConfig {
    workload_name: String,
    phases: Vec<PhaseConfig>,
    #[serde(default = "default_threshold")]
    adaptation_threshold: f64,
}

#[derive(Debug, Deserialize)]
struct PhaseConfig {
    name: String,
    workload: WorkloadConfig,
}

fn default_threshold() -> f64 { 0.9 }

fn main() -> Result<()> {
    env_logger::init();

    let config_str = fs::read_to_string("benchmarks/extended_adversarial.json")?;
    let ext_config: ExtendedConfig = serde_json::from_str(&config_str)?;

    let first_phase = &ext_config.phases[0];
    let mut runner = BenchmarkRunner::new(first_phase.workload.clone());
    runner.initialize();
    runner.total_ops_executed = 0;

    println!("=== Extended Adversarial Test ===");

    let mut all_snapshots: Vec<serde_json::Value> = Vec::new();
    let mut adaptation_latencies: Vec<(String, Option<usize>)> = Vec::new();

    for (i, phase) in ext_config.phases.iter().enumerate() {
        runner.current_phase_name = phase.name.clone();
        runner.config = phase.workload.clone();

        // Reset counters so each phase's utilisation curve starts from 0.
        if i > 0 {
            runner.metrics.reset();
        }

        let phase_start_op = runner.total_ops_executed;
        let snapshot_offset = runner.snapshots.len();
        runner.run();

        // Collect snapshots from this phase only.
        for (op_idx, report) in &runner.snapshots[snapshot_offset..] {
            all_snapshots.push(serde_json::json!({
                "operation": op_idx,
                "phase": report.phase_name,
                "factor_utilization": report.factor_utilization,
                "uaf": report.uaf,
                "structural_factors": report.structural_factor_count,
                "operational_factors": report.operational_factor_count,
            }));
        }

        // Adaptation latency: first snapshot in this phase where util >= threshold.
        if i > 0 {
            let threshold = ext_config.adaptation_threshold;
            let latency = runner.snapshots[snapshot_offset..]
                .iter()
                .find(|(_, rep)| rep.factor_utilization >= threshold)
                .map(|(op, _)| op - phase_start_op);
            adaptation_latencies.push((phase.name.clone(), latency));
        }
    }

    let snapshots_json = serde_json::to_string_pretty(&all_snapshots)?;
    fs::write("extended_adversarial_snapshots.json", snapshots_json)?;
    println!("Snapshots saved to extended_adversarial_snapshots.json");

    println!("\n=== Adaptation Latency (to reach {}% factor utilization) ===",
             (ext_config.adaptation_threshold * 100.0) as u32);
    for (name, lat) in &adaptation_latencies {
        match lat {
            Some(ops) => println!("Phase {}: {} operations", name, ops),
            None => println!("Phase {}: never recovered", name),
        }
    }

    runner.print_summary();
    Ok(())
}

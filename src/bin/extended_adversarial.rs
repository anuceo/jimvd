use anyhow::Result;
use jimvd::{benchmark::BenchmarkRunner, workload::WorkloadConfig};
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

    let config_path = std::env::args().nth(1)
        .unwrap_or_else(|| "benchmarks/extended_adversarial.json".to_string());
    let config_str = fs::read_to_string(&config_path)?;
    let ext_config: ExtendedConfig = serde_json::from_str(&config_str)?;

    println!("=== Extended Adversarial Test: {} ===\n", ext_config.workload_name);

    let first_phase = &ext_config.phases[0];
    let mut runner = BenchmarkRunner::new(first_phase.workload.clone());
    runner.initialize();
    runner.total_ops_executed = 0;

    let mut all_snapshots: Vec<serde_json::Value> = Vec::new();
    let mut adaptation_latencies: Vec<(String, Option<usize>)> = Vec::new();

    for (phase_idx, phase) in ext_config.phases.iter().enumerate() {
        println!("\n--- Phase {} ---", phase.name);
        runner.current_phase_name = phase.name.clone();
        runner.config = phase.workload.clone();

        let phase_start_op = runner.total_ops_executed;
        let snapshot_offset = runner.snapshots.len();
        runner.run();

        // Collect only this phase's new snapshots (avoid double-counting earlier phases).
        for (op_idx, report) in &runner.snapshots[snapshot_offset..] {
            all_snapshots.push(serde_json::json!({
                "operation":          op_idx,
                "phase":              report.phase_name,
                "factor_utilization": report.factor_utilization,
                "uaf":                report.uaf,
                "structural_factors": report.structural_factor_count,
                "operational_factors": report.operational_factor_count,
            }));
        }

        // Measure adaptation latency for all phases after the first.
        if phase_idx > 0 {
            let threshold = ext_config.adaptation_threshold;
            let latency = runner.snapshots[snapshot_offset..].iter()
                .find(|(_, rep)| rep.factor_utilization >= threshold)
                .map(|(op, _)| op - phase_start_op);
            adaptation_latencies.push((phase.name.clone(), latency));
        }
    }

    let snapshots_json = serde_json::to_string_pretty(&all_snapshots)?;
    fs::write("extended_adversarial_snapshots.json", &snapshots_json)?;
    println!("\nSnapshots saved to extended_adversarial_snapshots.json");

    println!(
        "\n=== Adaptation Latency (to reach {:.0}% factor utilization) ===",
        ext_config.adaptation_threshold * 100.0
    );
    for (name, lat) in &adaptation_latencies {
        match lat {
            Some(ops) => println!("  Phase {}: {} ops", name, ops),
            None      => println!("  Phase {}: never reached threshold", name),
        }
    }

    runner.print_summary();
    Ok(())
}

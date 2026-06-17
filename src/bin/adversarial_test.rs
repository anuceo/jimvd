use anyhow::Result;
use jimvd::benchmark::{run_julia_script, BenchmarkRunner};
use jimvd::workload::WorkloadConfig;
use serde_json::Value;
use std::fs;

fn main() -> Result<()> {
    env_logger::init();

    let config_str = fs::read_to_string("benchmarks/adversarial_config.json")?;
    let full_config: Value = serde_json::from_str(&config_str)?;

    let transition_at: usize = full_config["transition_at_ops"]
        .as_u64()
        .expect("transition_at_ops missing") as usize;

    let phase_a_config: WorkloadConfig =
        serde_json::from_value(full_config["phase_a"].clone())?;
    let phase_b_config: WorkloadConfig =
        serde_json::from_value(full_config["phase_b"].clone())?;

    // ── Phase A ──────────────────────────────────────────────────────────────
    println!("=== Phase A: Training on IAM attributes (Role / Region / Department) ===\n");
    let mut runner = BenchmarkRunner::new(phase_a_config);
    runner.initialize();

    // Reproducibility + covering-step metadata, recorded into every snapshot.
    let rng_seed = runner.rng_seed;
    let cover_build_secs = runner.cover_build_secs;
    let cover_factor_count = runner.cover_factor_count;
    println!(
        "[Meta] rng_seed={} cover_build_secs={:.4} cover_factor_count={}",
        rng_seed, cover_build_secs, cover_factor_count
    );

    runner.run();
    runner.print_summary();

    let snapshot_json = |op: usize, phase: &str, r: &jimvd::types::MetricsReport| -> Value {
        serde_json::json!({
            "operation": op,
            "phase": phase,
            "rng_seed": rng_seed,
            "cover_build_secs": cover_build_secs,
            "cover_factor_count": cover_factor_count,
            "factor_utilization": r.factor_utilization,
            "query_factor_utilization": r.query_factor_utilization,
            "query_factor_ops": r.query_factor_ops,
            "write_factor_ops": r.write_factor_ops,
            "row_ops": r.row_ops,
            "write_propagation_nodes": r.write_propagation_nodes,
            "uaf": r.uaf,
            "memory_bytes": r.memory_bytes,
            "storage_bytes": r.storage_bytes,
            "structural_factors": r.structural_factor_count,
            "operational_factors": r.operational_factor_count,
            "active_factors": r.active_factors,
            "evicted_factors": r.evicted_factors,
        })
    };

    // Collect Phase A snapshots before the config is swapped out.
    let mut all_snapshots: Vec<Value> = runner
        .snapshots
        .iter()
        .map(|(op, r)| snapshot_json(*op, "A", r))
        .collect();

    // ── Phase B ──────────────────────────────────────────────────────────────
    println!("\n=== Phase B: Switching to Clearance / Project / Office ===\n");
    runner.change_workload(phase_b_config);
    runner.run();
    runner.print_summary();

    let mut phase_b_snapshots: Vec<Value> = runner
        .snapshots
        .iter()
        // Offset so the timeline is continuous across both phases.
        .map(|(op, r)| snapshot_json(*op + transition_at, "B", r))
        .collect();

    all_snapshots.append(&mut phase_b_snapshots);

    // ── Write output ─────────────────────────────────────────────────────────
    let json_str = serde_json::to_string_pretty(&all_snapshots)?;
    fs::write("adversarial_snapshots.json", &json_str)?;

    println!(
        "\nAdversarial test complete — {} snapshots written to adversarial_snapshots.json",
        all_snapshots.len()
    );

    // Offline analysis — no-op if Julia is not installed.
    run_julia_script("plot_metrics.jl",    "adversarial_snapshots.json");
    run_julia_script("halflife_report.jl", "adversarial_snapshots.json");

    Ok(())
}

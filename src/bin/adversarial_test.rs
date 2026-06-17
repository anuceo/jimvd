use anyhow::Result;
use jimvd::benchmark::{run_julia_script, BenchmarkRunner};
use jimvd::workload::WorkloadConfig;
use serde_json::Value;
use std::fs;

fn main() -> Result<()> {
    env_logger::init();

    let config_str = fs::read_to_string("config/adversarial.json")?;
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
    runner.run();
    runner.print_summary();

    // Collect Phase A snapshots before the config is swapped out.
    let mut all_snapshots: Vec<Value> = runner
        .snapshots
        .iter()
        .map(|(op, r)| {
            serde_json::json!({
                "operation": op,
                "phase": "A",
                "factor_utilization": r.factor_utilization,
                "uaf": r.uaf,
                "structural_factors": r.structural_factor_count,
                "operational_factors": r.operational_factor_count,
                "active_factors": r.active_factors,
                "evicted_factors": r.evicted_factors,
            })
        })
        .collect();

    // ── Phase B ──────────────────────────────────────────────────────────────
    println!("\n=== Phase B: Switching to Clearance / Project / Office ===\n");
    runner.change_workload(phase_b_config);
    runner.run();
    runner.print_summary();

    let mut phase_b_snapshots: Vec<Value> = runner
        .snapshots
        .iter()
        .map(|(op, r)| {
            serde_json::json!({
                // Offset so the timeline is continuous across both phases.
                "operation": *op + transition_at,
                "phase": "B",
                "factor_utilization": r.factor_utilization,
                "uaf": r.uaf,
                "structural_factors": r.structural_factor_count,
                "operational_factors": r.operational_factor_count,
                "active_factors": r.active_factors,
                "evicted_factors": r.evicted_factors,
            })
        })
        .collect();

    all_snapshots.append(&mut phase_b_snapshots);

    // ── Write output ─────────────────────────────────────────────────────────
    let json_str = serde_json::to_string_pretty(&all_snapshots)?;
    fs::create_dir_all("data")?;
    fs::write("data/adversarial_snapshots.json", &json_str)?;

    println!(
        "\nAdversarial test complete — {} snapshots written to data/adversarial_snapshots.json",
        all_snapshots.len()
    );

    // Offline analysis — no-op if Julia is not installed.
    run_julia_script("plot_metrics.jl",    "data/adversarial_snapshots.json");
    run_julia_script("halflife_report.jl", "data/adversarial_snapshots.json");

    Ok(())
}

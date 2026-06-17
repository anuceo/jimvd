//! Scaling-wall benchmark: runs the same workload at increasing object counts
//! (up to 100,000 by default) and records the cost of the greedy covering step
//! alongside the standard metrics. The goal is to locate the scale at which
//! `GreedyCover::build_factors()` becomes the bottleneck.

use anyhow::Result;
use clap::Parser;
use jimvd::benchmark::BenchmarkRunner;
use jimvd::workload::WorkloadConfig;
use serde_json::Value;
use std::fs;

#[derive(Parser, Debug)]
#[command(about = "JimVD scaling-wall benchmark")]
struct Args {
    /// Maximum number of objects to scale up to.
    #[arg(long, default_value_t = 100_000)]
    max_scale: usize,

    /// Operations to run at each scale (kept modest so covering dominates).
    #[arg(long, default_value_t = 2_000)]
    operations: usize,

    /// Master RNG seed for reproducibility.
    #[arg(long, default_value_t = 0)]
    seed: u64,

    /// Base workload config to template from.
    #[arg(long, default_value = "benchmarks/workload_iam.json")]
    config: String,

    /// Output snapshot file.
    #[arg(long, default_value = "scaling_snapshots.json")]
    out: String,
}

fn scales_up_to(max_scale: usize) -> Vec<usize> {
    let ladder = [1_000usize, 5_000, 10_000, 50_000, 100_000];
    let mut scales: Vec<usize> = ladder.iter().copied().filter(|s| *s <= max_scale).collect();
    if scales.last() != Some(&max_scale) {
        scales.push(max_scale);
    }
    scales.retain(|s| *s > 0);
    scales.dedup();
    scales
}

fn load_config(path: &str, scale: usize, operations: usize, seed: u64) -> Result<WorkloadConfig> {
    let config_str = fs::read_to_string(path)?;
    let mut config: WorkloadConfig = serde_json::from_str(&config_str)?;
    config.rng_seed = seed;
    for spec in config.tables.values_mut() {
        spec.initial_objects = scale;
    }
    config.run_options.total_operations = operations;
    config.run_options.warmup_ops = 0;
    config.run_options.metrics_interval_ops = operations.max(1);
    Ok(config)
}

fn main() -> Result<()> {
    env_logger::init();
    let args = Args::parse();

    let scales = scales_up_to(args.max_scale);
    println!(
        "=== Scaling wall: scales={:?} ops/scale={} seed={} ===\n",
        scales, args.operations, args.seed
    );

    let mut snapshots: Vec<Value> = Vec::new();

    for scale in scales {
        println!("--- Scale: {} objects ---", scale);
        let config = load_config(&args.config, scale, args.operations, args.seed)?;
        let mut runner = BenchmarkRunner::new(config);
        runner.initialize();
        runner.run();
        let r = runner.report();

        println!(
            "[Scale {:>7}]  cover_build={:.4}s  factors={}  query_util={:.1}%  uaf={:.2}  mem={}B  storage={}B",
            scale,
            runner.cover_build_secs,
            runner.cover_factor_count,
            r.query_factor_utilization * 100.0,
            r.uaf,
            r.memory_bytes,
            r.storage_bytes,
        );

        snapshots.push(serde_json::json!({
            "scale": scale,
            "rng_seed": runner.rng_seed,
            "cover_build_secs": runner.cover_build_secs,
            "cover_factor_count": runner.cover_factor_count,
            "operations": args.operations,
            "total_queries": r.total_queries,
            "query_factor_ops": r.query_factor_ops,
            "write_factor_ops": r.write_factor_ops,
            "row_ops": r.row_ops,
            "factor_utilization": r.factor_utilization,
            "query_factor_utilization": r.query_factor_utilization,
            "uaf": r.uaf,
            "write_propagation_nodes": r.write_propagation_nodes,
            "structural_factors": r.structural_factor_count,
            "operational_factors": r.operational_factor_count,
            "memory_bytes": r.memory_bytes,
            "storage_bytes": r.storage_bytes,
        }));
        println!();
    }

    let json_str = serde_json::to_string_pretty(&snapshots)?;
    fs::write(&args.out, &json_str)?;
    println!("Scaling wall complete — {} scale points written to {}", snapshots.len(), args.out);

    Ok(())
}

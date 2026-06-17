use anyhow::Result;
use jimvd::benchmark::{run_julia_script, BenchmarkRunner};
use jimvd::workload::WorkloadConfig;
use std::fs;

fn main() -> Result<()> {
    let config_path = std::env::args().nth(1)
        .unwrap_or_else(|| "config/join_test.json".to_string());
    let config_str = fs::read_to_string(&config_path)?;
    let config: WorkloadConfig = serde_json::from_str(&config_str)?;

    println!("=== Join Benchmark: {} ===", config.workload_name);
    println!("{}\n", config.description);

    let mut runner = BenchmarkRunner::new(config);
    runner.initialize();
    runner.run();
    runner.print_summary();

    let json_str = serde_json::to_string_pretty(&runner.snapshots)?;
    fs::create_dir_all("data")?;
    fs::write("data/join_snapshots.json", &json_str)?;
    println!("\n{} snapshots written to data/join_snapshots.json", runner.snapshots.len());

    run_julia_script("plot_metrics.jl",    "data/join_snapshots.json");
    run_julia_script("halflife_report.jl", "data/join_snapshots.json");

    Ok(())
}

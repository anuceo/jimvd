use anyhow::Result;
use jimvd::benchmark::BenchmarkRunner;
use jimvd::workload::WorkloadConfig;
use std::fs;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();

    let config_str = fs::read_to_string("benchmarks/workload_iam.json")?;
    let config: WorkloadConfig = serde_json::from_str(&config_str)?;
    println!("Loaded workload: {:?}\n", config.workload_name);

    let mut runner = BenchmarkRunner::new(config);
    runner.initialize();
    runner.run();
    runner.print_summary();

    Ok(())
}
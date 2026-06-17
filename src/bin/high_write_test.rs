use anyhow::Result;
use jimvd::{
    benchmark::BenchmarkRunner,
    workload::WorkloadConfig,
};
use std::fs;

fn main() -> Result<()> {
    env_logger::init();

    let config_path = std::env::args().nth(1)
        .unwrap_or_else(|| "benchmarks/high_write.json".to_string());
    let config_str = fs::read_to_string(&config_path)?;
    let config: WorkloadConfig = serde_json::from_str(&config_str)?;
    println!("Loaded high-write workload: {}\n", config.workload_name);

    let mut runner = BenchmarkRunner::new(config);
    runner.initialize();
    runner.run();
    runner.print_summary();

    if !runner.fanout_log.is_empty() {
        let mut sorted = runner.fanout_log.clone();
        sorted.sort_unstable();
        let len = sorted.len();
        let p50 = sorted[len / 2];
        let p95 = sorted[((len as f64 * 0.95) as usize).min(len - 1)];
        let p99 = sorted[((len as f64 * 0.99) as usize).min(len - 1)];
        let sum: u64 = sorted.iter().sum();
        let mean = sum as f64 / len as f64;

        println!("\n=== Propagation Fanout (per delta) ===");
        println!("Count:           {}", len);
        println!("Min:             {}", sorted[0]);
        println!("Max:             {}", sorted[len - 1]);
        println!("Mean:            {:.2}", mean);
        println!("Median (p50):    {}", p50);
        println!("95th pct (p95):  {}", p95);
        println!("99th pct (p99):  {}", p99);
    }

    Ok(())
}

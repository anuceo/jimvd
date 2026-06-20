use anyhow::Result;
use clap::{Parser, Subcommand};
use data_generator::CorrelationConfig;
use serde::Deserialize;
use workload_generator::{Phase, WorkloadConfig};

#[derive(Parser)]
#[command(name = "runner", about = "JimVD benchmark runner")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    ScalingWall {
        #[arg(long, default_value = "100000")]
        max_scale: usize,
    },
    DriftSim {
        #[arg(long, default_value = "1000000")]
        ops_per_phase: usize,
    },
    JoinExplosion {
        #[arg(long, default_value = "10000")]
        orders: usize,
    },
    Evolution {
        #[arg(long, default_value = "10000000")]
        total_ops: usize,
    },
    Full,
    /// The single definitive benchmark: 1M users, 100M ops, 6 workload shifts, JimVD vs DuckDB.
    MainEvent {
        #[arg(long, default_value = "1000000")]
        users: usize,
        /// Ops per phase (6 phases → total = 6 × ops_per_phase).
        #[arg(long, default_value = "16666666")]
        ops_per_phase: usize,
        /// How often to capture a metrics snapshot (in operations).
        #[arg(long, default_value = "1000000")]
        snapshot_interval: usize,
        /// Factor utilization threshold for measuring adaptation latency.
        #[arg(long, default_value = "0.9")]
        adaptation_threshold: f64,
    },
    /// Quick smoke test: 1K users, 6 × 2K ops — verifies the suite runs end-to-end in seconds.
    QuickSmoke,
}

#[derive(Deserialize, Default)]
struct DatasetToml {
    size: Option<usize>,
}

#[derive(Deserialize, Default)]
struct CorrelationToml {
    role_department_bias:  Option<f64>,
    region_clearance_bias: Option<f64>,
    tenant_role_bias:      Option<f64>,
}

#[derive(Deserialize, Default)]
struct WorkloadToml {
    read_ratio:       Option<f64>,
    write_ratio:      Option<f64>,
    join_ratio:       Option<f64>,
    total_operations: Option<usize>,
}

#[derive(Deserialize, Default)]
struct BenchConfig {
    dataset:     Option<DatasetToml>,
    correlation: Option<CorrelationToml>,
    workload:    Option<WorkloadToml>,
}

fn load_config(path: &str) -> BenchConfig {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| toml::from_str(&s).ok())
        .unwrap_or_default()
}

fn make_correlation(ct: Option<CorrelationToml>) -> CorrelationConfig {
    let def = CorrelationConfig::default();
    match ct {
        None => def,
        Some(c) => CorrelationConfig {
            role_department_bias:  c.role_department_bias.unwrap_or(def.role_department_bias),
            region_clearance_bias: c.region_clearance_bias.unwrap_or(def.region_clearance_bias),
            tenant_role_bias:      c.tenant_role_bias.unwrap_or(def.tenant_role_bias),
        },
    }
}

fn make_workload(wt: Option<WorkloadToml>, default_total: usize) -> WorkloadConfig {
    match wt {
        None => WorkloadConfig {
            read_ratio:       0.6,
            write_ratio:      0.4,
            join_ratio:       0.0,
            total_operations: default_total,
            rng_seed:         0,
        },
        Some(w) => WorkloadConfig {
            read_ratio:       w.read_ratio.unwrap_or(0.6),
            write_ratio:      w.write_ratio.unwrap_or(0.4),
            join_ratio:       w.join_ratio.unwrap_or(0.0),
            total_operations: w.total_operations.unwrap_or(default_total),
            rng_seed:         0,
        },
    }
}

fn main() -> Result<()> {
    env_logger::init();
    let cli = Cli::parse();
    std::fs::create_dir_all("results")?;

    match cli.command {
        Command::ScalingWall { max_scale } => {
            log::info!("Running ScalingWall, max_scale={}", max_scale);
            let cfg = load_config("configs/iam_1m.toml");
            let corr = make_correlation(cfg.correlation);
            let wl = make_workload(cfg.workload, 10_000);
            let scale = cfg.dataset.and_then(|d| d.size).unwrap_or(max_scale);
            let mut runner = jimvd_runner::JimvdRunner::new(3, 10_000);
            let results = benchmark_orchestrator::scaling_wall::run_scaling_wall(
                &mut runner,
                &corr,
                &wl,
                Phase::IAM,
                scale,
            );
            let rows: Vec<(&str, usize, &metrics::Metrics)> = results
                .iter()
                .map(|r| (r.runner_name.as_str(), r.scale, &r.metrics))
                .collect();
            report_generator::write_csv("results/scaling_wall.csv", &rows)?;
            let json_data: Vec<serde_json::Value> = results.iter().map(|r| {
                serde_json::json!({
                    "scale": r.scale,
                    "runner": r.runner_name,
                    "p50_us": r.metrics.p50_latency_us,
                    "factor_util": r.metrics.factor_utilization,
                    "uaf": r.metrics.uaf,
                })
            }).collect();
            report_generator::write_json("results/scaling_wall.json", &json_data)?;
            let series: Vec<(&str, Vec<(usize, f64)>)> = vec![(
                "jimvd",
                results.iter().map(|r| (r.scale, r.metrics.uaf)).collect(),
            )];
            report_generator::plot_uaf_vs_scale("results/uaf_vs_scale.png", &series)?;

            println!("=== Scaling Wall Results ===");
            println!("{:<12} {:<12} {:<10} {:<10} {:<8}", "Runner", "Scale", "P50(µs)", "P99(µs)", "UAF");
            for r in &results {
                println!("{:<12} {:<12} {:<10} {:<10} {:.4}",
                    r.runner_name, r.scale,
                    r.metrics.p50_latency_us, r.metrics.p99_latency_us,
                    r.metrics.uaf);
            }

            // Also run DuckDB for comparison if available
            if let Ok(mut ddb) = duckdb_runner::DuckdbRunner::new() {
                let ddb_results = benchmark_orchestrator::scaling_wall::run_scaling_wall(
                    &mut ddb, &corr, &wl, Phase::IAM, scale,
                );
                println!("\n=== DuckDB Scaling Wall ===");
                println!("{:<12} {:<12} {:<10} {:<10}", "Runner", "Scale", "P50(µs)", "P99(µs)");
                for r in &ddb_results {
                    println!("{:<12} {:<12} {:<10} {:<10}",
                        r.runner_name, r.scale,
                        r.metrics.p50_latency_us, r.metrics.p99_latency_us);
                }
                let ddb_json: Vec<serde_json::Value> = ddb_results.iter().map(|r| serde_json::json!({
                    "scale": r.scale, "runner": r.runner_name,
                    "p50_us": r.metrics.p50_latency_us, "p99_us": r.metrics.p99_latency_us,
                })).collect();
                report_generator::write_json("results/scaling_wall_duckdb.json", &ddb_json)?;
            }
        }

        Command::DriftSim { ops_per_phase } => {
            log::info!("Running DriftSim, ops_per_phase={}", ops_per_phase);
            let cfg = load_config("configs/iam_1m.toml");
            let corr = make_correlation(cfg.correlation);
            let dataset_size = cfg.dataset.and_then(|d| d.size).unwrap_or(10_000);
            let users = data_generator::generate_users(dataset_size, &corr);
            let wl = make_workload(cfg.workload, ops_per_phase);
            let mut runner = jimvd_runner::JimvdRunner::new(3, 10_000);
            let results = benchmark_orchestrator::drift_simulator::run_drift_simulation(
                &mut runner, &users, ops_per_phase, &wl,
            );
            let json_data: Vec<serde_json::Value> = results.iter().map(|r| {
                serde_json::json!({
                    "phase_index": r.phase_index,
                    "factor_util": r.metrics.factor_utilization,
                    "uaf": r.metrics.uaf,
                    "adaptation_latency_ops": r.adaptation_latency_ops,
                })
            }).collect();
            report_generator::write_json("results/drift_sim.json", &json_data)?;
            println!("=== Drift Simulation Results ===");
            for r in &results {
                println!("Phase {:?}: util={:.3} uaf={:.3} adapt={:?}",
                    r.phase, r.metrics.factor_utilization, r.metrics.uaf,
                    r.adaptation_latency_ops);
            }
        }

        Command::JoinExplosion { orders } => {
            log::info!("Running JoinExplosion, orders={}", orders);
            let cfg = load_config("configs/mixed_workload.toml");
            let products = cfg.dataset.and_then(|d| d.size).unwrap_or(1000);
            let mut runner = jimvd_runner::JimvdRunner::new(3, 10_000);
            let results = benchmark_orchestrator::join_explosion::run_join_explosion(
                &mut runner,
                orders,
                products,
                benchmark_orchestrator::join_explosion::JOIN_FANOUTS,
            );
            let json_data: Vec<serde_json::Value> = results.iter().map(|r| {
                serde_json::json!({
                    "fanout": r.avg_items_per_order,
                    "runner": r.runner_name,
                    "p99_us": r.metrics.p99_latency_us,
                    "fallback_rate": r.fallback_rate,
                })
            }).collect();
            report_generator::write_json("results/join_explosion.json", &json_data)?;
            println!("=== Join Explosion Results ===");
            for r in &results {
                println!("Fanout {:>4}: runner={} p99={}µs fallback={:.2}",
                    r.avg_items_per_order, r.runner_name,
                    r.metrics.p99_latency_us, r.fallback_rate);
            }
        }

        Command::Evolution { total_ops } => {
            log::info!("Running Evolution, total_ops={}", total_ops);
            let cfg = load_config("configs/mixed_workload.toml");
            let corr = make_correlation(cfg.correlation);
            let dataset_size = cfg.dataset.and_then(|d| d.size).unwrap_or(10_000);
            let wt = cfg.workload;
            let read_ratio  = wt.as_ref().and_then(|w| w.read_ratio).unwrap_or(0.6);
            let write_ratio = wt.as_ref().and_then(|w| w.write_ratio).unwrap_or(0.4);
            let users = data_generator::generate_users(dataset_size, &corr);
            let mut runner = jimvd_runner::JimvdRunner::new(3, 10_000);
            let snapshots = benchmark_orchestrator::evolution::run_long_term_evolution(
                &mut runner, &users, total_ops, 1_000_000, 2_000_000, read_ratio, write_ratio,
            );
            let evo_points: Vec<report_generator::EvolutionPoint> = snapshots
                .iter()
                .map(|s| report_generator::EvolutionPoint {
                    operations:         s.operations,
                    factor_utilization: s.metrics.factor_utilization,
                })
                .collect();
            report_generator::plot_factor_utilization_over_time(
                "results/factor_util_evolution.png", &evo_points,
            )?;
            let json_data: Vec<serde_json::Value> = snapshots.iter().map(|s| {
                serde_json::json!({
                    "operations": s.operations,
                    "factor_util": s.metrics.factor_utilization,
                    "uaf": s.metrics.uaf,
                })
            }).collect();
            report_generator::write_json("results/evolution.json", &json_data)?;
            println!("=== Evolution: {} snapshots captured ===", snapshots.len());
        }

        Command::Full => {
            log::info!("Running Full benchmark suite");
            let corr = CorrelationConfig::default();
            let wl = WorkloadConfig {
                read_ratio: 0.6, write_ratio: 0.4, join_ratio: 0.0,
                total_operations: 5_000, rng_seed: 0,
            };

            // ── 1. Scaling Wall ──────────────────────────────────────────
            println!("\n=== [1/3] Scaling Wall ===");
            let mut runner = jimvd_runner::JimvdRunner::new(3, 10_000);
            let sw_results = benchmark_orchestrator::scaling_wall::run_scaling_wall(
                &mut runner, &corr, &wl, Phase::IAM, 10_000,
            );
            let sw_rows: Vec<(&str, usize, &metrics::Metrics)> = sw_results
                .iter().map(|r| (r.runner_name.as_str(), r.scale, &r.metrics)).collect();
            report_generator::write_csv("results/scaling_wall.csv", &sw_rows)?;
            let sw_json: Vec<serde_json::Value> = sw_results.iter().map(|r| serde_json::json!({
                "scale": r.scale, "runner": r.runner_name,
                "p50_us": r.metrics.p50_latency_us, "p95_us": r.metrics.p95_latency_us,
                "p99_us": r.metrics.p99_latency_us,
                "factor_utilization": r.metrics.factor_utilization, "uaf": r.metrics.uaf,
            })).collect();
            report_generator::write_json("results/scaling_wall.json", &sw_json)?;
            let sw_series: Vec<(&str, Vec<(usize, f64)>)> = vec![(
                "jimvd", sw_results.iter().map(|r| (r.scale, r.metrics.uaf)).collect(),
            )];
            report_generator::plot_uaf_vs_scale("results/uaf_vs_scale.png", &sw_series)?;
            if let Ok(mut ddb) = duckdb_runner::DuckdbRunner::new() {
                let ddb_sw = benchmark_orchestrator::scaling_wall::run_scaling_wall(
                    &mut ddb, &corr, &wl, Phase::IAM, 10_000,
                );
                for r in &ddb_sw {
                    println!("DuckDB {:<12} {:<10} {:<10} {:.4}",
                        r.scale, r.metrics.p50_latency_us, r.metrics.p99_latency_us, r.metrics.uaf);
                }
            }
            println!("{:<12} {:<10} {:<10} {:<10} {:<8}", "Scale", "P50(µs)", "P95(µs)", "P99(µs)", "UAF");
            for r in &sw_results {
                println!("{:<12} {:<10} {:<10} {:<10} {:.4}",
                    r.scale, r.metrics.p50_latency_us, r.metrics.p95_latency_us,
                    r.metrics.p99_latency_us, r.metrics.uaf);
            }

            // ── 2. Drift Simulation ─────────────────────────────────────
            println!("\n=== [2/3] Workload Drift Simulation ===");
            let users = data_generator::generate_users(5_000, &corr);
            let mut runner2 = jimvd_runner::JimvdRunner::new(3, 10_000);
            let drift_results = benchmark_orchestrator::drift_simulator::run_drift_simulation(
                &mut runner2, &users, 2_000, &wl,
            );
            let drift_json: Vec<serde_json::Value> = drift_results.iter().map(|r| serde_json::json!({
                "phase_index": r.phase_index,
                "factor_utilization": r.metrics.factor_utilization,
                "uaf": r.metrics.uaf,
                "adaptation_latency_ops": r.adaptation_latency_ops,
            })).collect();
            report_generator::write_json("results/drift_sim.json", &drift_json)?;
            println!("{:<6} {:<14} {:<14} {:>14}", "Phase", "Type", "FactorUtil", "AdaptLatency");
            for r in &drift_results {
                println!("{:<6} {:<14} {:<14.4} {:>14}",
                    r.phase_index, format!("{:?}", r.phase),
                    r.metrics.factor_utilization,
                    r.adaptation_latency_ops.map(|n| n.to_string()).unwrap_or_else(|| "—".into()));
            }

            // ── 3. Join Explosion ───────────────────────────────────────
            println!("\n=== [3/3] Join Explosion ===");
            let mut runner3 = jimvd_runner::JimvdRunner::new(3, 10_000);
            let join_results = benchmark_orchestrator::join_explosion::run_join_explosion(
                &mut runner3, 200, 200,
                benchmark_orchestrator::join_explosion::JOIN_FANOUTS,
            );
            let join_json: Vec<serde_json::Value> = join_results.iter().map(|r| serde_json::json!({
                "fanout": r.avg_items_per_order, "runner": r.runner_name,
                "p50_us": r.metrics.p50_latency_us, "p99_us": r.metrics.p99_latency_us,
                "fallback_rate": r.fallback_rate,
            })).collect();
            report_generator::write_json("results/join_explosion.json", &join_json)?;
            println!("{:<8} {:<10} {:<10} {:<10}", "Fanout", "P50(µs)", "P99(µs)", "FallbackRate");
            for r in &join_results {
                println!("{:<8} {:<10} {:<10} {:.4}",
                    r.avg_items_per_order, r.metrics.p50_latency_us,
                    r.metrics.p99_latency_us, r.fallback_rate);
            }

            println!("\nFull benchmark complete — results written to results/");
        }

        Command::MainEvent { users, ops_per_phase, snapshot_interval, adaptation_threshold } => {
            let total_ops = 6 * ops_per_phase;
            println!("=== Main Event ===");
            println!("  users          : {}", users);
            println!("  ops_per_phase  : {} ({} phases → {} total)", ops_per_phase, 6, total_ops);
            println!("  snapshot_every : {} ops", snapshot_interval);
            println!("  adapt_threshold: {:.0}%", adaptation_threshold * 100.0);

            let corr = CorrelationConfig::default();
            let dataset = data_generator::generate_users(users, &corr);
            let mut all_results: Vec<benchmark_orchestrator::main_event::MainEventResult> = Vec::new();
            let mut all_rows: Vec<report_generator::PhaseMetricRow> = Vec::new();

            // ── JimVD ──────────────────────────────────────────────────────
            {
                let mut runner = jimvd_runner::JimvdRunner::new(3, 10_000);
                println!("\n--- Running JimVD ({} users) ---", dataset.len());
                let result = benchmark_orchestrator::main_event::run_main_event(
                    &mut runner, &dataset, ops_per_phase, snapshot_interval, adaptation_threshold,
                );
                for s in &result.snapshots {
                    all_rows.push(report_generator::PhaseMetricRow {
                        runner:             result.runner_name.clone(),
                        phase:              s.phase.clone(),
                        phase_index:        s.phase_index,
                        ops_into_phase:     s.ops_into_phase,
                        total_ops:          s.total_ops,
                        p50_us:             s.metrics.p50_latency_us,
                        p99_us:             s.metrics.p99_latency_us,
                        factor_utilization: s.metrics.factor_utilization,
                        uaf:                s.metrics.uaf,
                        memory_bytes:       s.metrics.memory_peak_bytes,
                        storage_bytes:      s.metrics.storage_bytes,
                    });
                }
                println!("[JimVD] completed in {:.1}s", result.total_elapsed_secs);
                all_results.push(result);
            }

            // ── DuckDB ─────────────────────────────────────────────────────
            if let Ok(mut runner) = duckdb_runner::DuckdbRunner::new() {
                println!("\n--- Running DuckDB ({} users) ---", dataset.len());
                let result = benchmark_orchestrator::main_event::run_main_event(
                    &mut runner, &dataset, ops_per_phase, snapshot_interval, adaptation_threshold,
                );
                for s in &result.snapshots {
                    all_rows.push(report_generator::PhaseMetricRow {
                        runner:             result.runner_name.clone(),
                        phase:              s.phase.clone(),
                        phase_index:        s.phase_index,
                        ops_into_phase:     s.ops_into_phase,
                        total_ops:          s.total_ops,
                        p50_us:             s.metrics.p50_latency_us,
                        p99_us:             s.metrics.p99_latency_us,
                        factor_utilization: s.metrics.factor_utilization,
                        uaf:                s.metrics.uaf,
                        memory_bytes:       s.metrics.memory_peak_bytes,
                        storage_bytes:      s.metrics.storage_bytes,
                    });
                }
                println!("[DuckDB] completed in {:.1}s", result.total_elapsed_secs);
                all_results.push(result);
            } else {
                println!("\n[DuckDB] not available — skipping");
            }

            // ── Write outputs ──────────────────────────────────────────────
            report_generator::write_json("results/main_event.json", &all_results)?;
            report_generator::write_main_event_csv("results/main_event.csv", &all_rows)?;

            // Factor utilization plot — one series per engine.
            let series: Vec<(&str, Vec<(usize, f64)>)> = all_results.iter()
                .map(|r| {
                    let pts: Vec<(usize, f64)> = r.snapshots.iter()
                        .map(|s| (s.total_ops, s.metrics.factor_utilization))
                        .collect();
                    (r.runner_name.as_str(), pts)
                })
                .collect();
            report_generator::plot_multi_engine_factor_util(
                "results/main_event_factor_util.png", &series,
            )?;

            // Adaptation latency plot — one bar per (engine, phase).
            let bars: Vec<(String, Option<usize>)> = all_results.iter()
                .flat_map(|r| r.adaptation_latencies.iter().map(move |a| {
                    (format!("{}:{}", r.runner_name, a.phase), a.latency_ops)
                }))
                .collect();
            let bar_refs: Vec<(&str, Option<usize>)> =
                bars.iter().map(|(l, v)| (l.as_str(), *v)).collect();
            report_generator::plot_adaptation_latency(
                "results/main_event_adaptation.png", &bar_refs, ops_per_phase,
            )?;

            // ── Console summary ────────────────────────────────────────────
            println!("\n=== Main Event Summary ===");
            for result in &all_results {
                println!("\n[{}] total elapsed: {:.1}s", result.runner_name, result.total_elapsed_secs);
                println!("{:<12} {:<14} {:<10} {:<10} {:<12}",
                    "Phase", "AdaptLatency", "FinalUtil", "FinalUAF", "P99(µs)");
                for (al, snap) in result.adaptation_latencies.iter()
                    .zip(result.snapshots.iter().filter(|s| s.ops_into_phase == ops_per_phase))
                {
                    println!("{:<12} {:<14} {:<10.3} {:<10.3} {:<12}",
                        al.phase,
                        al.latency_ops.map(|n| n.to_string()).unwrap_or_else(|| "never".to_string()),
                        snap.metrics.factor_utilization,
                        snap.metrics.uaf,
                        snap.metrics.p99_latency_us);
                }
            }
            println!("\nOutputs written to results/");
        }

        Command::QuickSmoke => {
            println!("=== Quick Smoke Test ===");
            const SMOKE_USERS: usize        = 1_000;
            const SMOKE_OPS_PER_PHASE: usize = 2_000;
            const SMOKE_SNAPSHOT: usize     = 500;

            let corr = CorrelationConfig::default();
            let dataset = data_generator::generate_users(SMOKE_USERS, &corr);
            let mut passed = 0usize;
            let mut failed = 0usize;

            macro_rules! smoke {
                ($label:expr, $runner:expr) => {{
                    let mut r = $runner;
                    let result = benchmark_orchestrator::main_event::run_main_event(
                        &mut r, &dataset, SMOKE_OPS_PER_PHASE, SMOKE_SNAPSHOT, 0.5,
                    );
                    if result.snapshots.is_empty() {
                        println!("  [FAIL] {} — no snapshots produced", $label);
                        failed += 1;
                    } else {
                        let last = result.snapshots.last().unwrap();
                        println!("  [OK]   {} — util={:.3} uaf={:.3} elapsed={:.2}s",
                            $label, last.metrics.factor_utilization,
                            last.metrics.uaf, result.total_elapsed_secs);
                        passed += 1;
                    }
                }};
            }

            smoke!("JimVD", jimvd_runner::JimvdRunner::new(3, 10_000));

            if let Ok(ddb) = duckdb_runner::DuckdbRunner::new() {
                smoke!("DuckDB", ddb);
            } else {
                println!("  [SKIP] DuckDB — feature not available");
            }

            println!("\nSmoke test: {}/{} engines passed", passed, passed + failed);
            if failed > 0 {
                std::process::exit(1);
            }
        }
    }

    Ok(())
}

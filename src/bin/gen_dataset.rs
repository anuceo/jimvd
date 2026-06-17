//! Deterministically materialise a dataset (CSV) and an engine-agnostic
//! operation log (JSONL) from a workload config. Both the JimVD replay and the
//! DuckDB runner consume these files so the two engines execute identical work.

use anyhow::{Context, Result};
use clap::Parser;
use jimvd::benchmark::{generate_props, sample_attr_value, sample_eq_value};
use jimvd::workload::{QueryTemplate, WorkloadConfig};
use rand::rngs::StdRng;
use rand::{RngExt, SeedableRng};
use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(about = "Generate CSV + operation log for multi-engine comparison")]
struct Args {
    /// Workload config to template from.
    #[arg(long, default_value = "benchmarks/workload_iam.json")]
    config: String,

    /// Override the per-table initial object count (0 = use config value).
    #[arg(long, default_value_t = 0)]
    scale: usize,

    /// Number of operations to emit in the op log.
    #[arg(long, default_value_t = 2_000)]
    operations: usize,

    /// Master RNG seed.
    #[arg(long, default_value_t = 0)]
    seed: u64,

    /// Output directory for employees.csv / oplog.jsonl / meta.json.
    #[arg(long, default_value = "data")]
    out_dir: String,

    /// If the config is a multi-phase file (e.g. the adversarial config with
    /// `phase_a`/`phase_b`), extract this nested phase as the workload.
    #[arg(long, default_value = "")]
    phase: String,
}

fn main() -> Result<()> {
    let args = Args::parse();

    let config_str = fs::read_to_string(&args.config)
        .with_context(|| format!("reading config {}", args.config))?;
    let mut config: WorkloadConfig = if args.phase.is_empty() {
        serde_json::from_str(&config_str)?
    } else {
        let full: serde_json::Value = serde_json::from_str(&config_str)?;
        let phase = full
            .get(&args.phase)
            .with_context(|| format!("config has no phase '{}'", args.phase))?
            .clone();
        serde_json::from_value(phase)?
    };
    config.rng_seed = args.seed;
    if args.scale > 0 {
        for spec in config.tables.values_mut() {
            spec.initial_objects = args.scale;
        }
    }

    let out_dir = PathBuf::from(&args.out_dir);
    fs::create_dir_all(&out_dir)?;

    // Stable column order: every categorical + continuous attribute, sorted.
    let mut columns: BTreeSet<String> = BTreeSet::new();
    for spec in config.tables.values() {
        columns.extend(spec.attributes.keys().cloned());
        columns.extend(spec.continuous.keys().cloned());
    }
    let columns: Vec<String> = columns.into_iter().collect();

    let mut rng = StdRng::seed_from_u64(args.seed);

    // ── 1. Initial data → CSV ────────────────────────────────────────────────
    let csv_path = out_dir.join("employees.csv");
    let mut csv = fs::File::create(&csv_path)?;
    write!(csv, "id")?;
    for c in &columns {
        write!(csv, ",{}", c)?;
    }
    writeln!(csv)?;

    let mut next_id: u32 = 0;
    for spec in config.tables.values() {
        for _ in 0..spec.initial_objects {
            let id = next_id;
            next_id += 1;
            let props = generate_props(spec, &mut rng);
            write!(csv, "{}", id)?;
            for c in &columns {
                match props.get(c) {
                    Some(v) => write!(csv, ",{}", csv_escape(v))?,
                    None => write!(csv, ",")?, // NULL = empty cell
                }
            }
            writeln!(csv)?;
        }
    }
    let initial_objects = next_id as usize;

    // ── 2. Operation log → JSONL ─────────────────────────────────────────────
    let oplog_path = out_dir.join("oplog.jsonl");
    let mut oplog = fs::File::create(&oplog_path)?;

    let write_rate = config.write_mix.insert_rate
        + config.write_mix.update_rate
        + config.write_mix.delete_rate;
    let query_weight: u32 = config.query_mix.iter().map(|t| t.weight()).sum();

    // Single-table assumption for op sampling (matches the bundled configs).
    let spec = config
        .tables
        .values()
        .next()
        .context("config has no tables")?;

    for _ in 0..args.operations {
        let op = if rng.random::<f64>() < write_rate {
            gen_write_op(&config, spec, &mut next_id, &mut rng)
        } else {
            gen_query_op(&config, spec, query_weight, &mut rng)
        };
        if let Some(op) = op {
            writeln!(oplog, "{}", serde_json::to_string(&op)?)?;
        }
    }

    // ── 3. Metadata ──────────────────────────────────────────────────────────
    let meta = serde_json::json!({
        "config": args.config,
        "seed": args.seed,
        "scale": args.scale,
        "initial_objects": initial_objects,
        "operations": args.operations,
        "columns": columns,
    });
    fs::write(out_dir.join("meta.json"), serde_json::to_string_pretty(&meta)?)?;

    println!(
        "Wrote {} ({} rows), {} ({} ops), and meta.json to {}/",
        csv_path.display(),
        initial_objects,
        oplog_path.display(),
        args.operations,
        args.out_dir
    );

    Ok(())
}

fn gen_query_op(
    config: &WorkloadConfig,
    spec: &jimvd::workload::TableSpec,
    total_weight: u32,
    rng: &mut StdRng,
) -> Option<serde_json::Value> {
    if total_weight == 0 {
        return None;
    }
    let pick = rng.random_range(0..total_weight);
    let mut cumulative = 0u32;
    for template in &config.query_mix {
        cumulative += template.weight();
        if pick < cumulative {
            return Some(match template {
                QueryTemplate::Eq { attribute, values, hot_values, .. } => {
                    let value = sample_eq_value(values, hot_values, rng);
                    serde_json::json!({"op": "query_eq", "attribute": attribute, "value": value})
                }
                QueryTemplate::And { attributes, .. } => {
                    let filters: Vec<serde_json::Value> = attributes
                        .iter()
                        .map(|a| {
                            let v = sample_attr_value(spec, a, rng)
                                .unwrap_or_else(|| "unknown".to_string());
                            serde_json::json!({"attribute": a, "value": v})
                        })
                        .collect();
                    serde_json::json!({"op": "query_and", "filters": filters})
                }
                QueryTemplate::Or { attributes, values, .. } => {
                    serde_json::json!({
                        "op": "query_or",
                        "attribute": attributes.first().cloned().unwrap_or_default(),
                        "values": values,
                    })
                }
            });
        }
    }
    None
}

fn gen_write_op(
    config: &WorkloadConfig,
    spec: &jimvd::workload::TableSpec,
    next_id: &mut u32,
    rng: &mut StdRng,
) -> Option<serde_json::Value> {
    let ins = config.write_mix.insert_rate;
    let upd = config.write_mix.update_rate;
    let del = config.write_mix.delete_rate;
    let total = ins + upd + del;
    if total == 0.0 {
        return None;
    }
    let roll = rng.random::<f64>() * total;

    if roll < ins {
        let id = *next_id;
        *next_id += 1;
        let props = generate_props(spec, rng);
        Some(serde_json::json!({"op": "insert", "id": id, "values": props}))
    } else if roll < ins + upd {
        if *next_id == 0 {
            return None;
        }
        let id = rng.random_range(0..*next_id);
        let attrs = &config.write_mix.attributes;
        if attrs.is_empty() {
            return None;
        }
        let attr =
            jimvd::benchmark::select_mutation_attribute(attrs, &config.write_mix.attribute_weights, rng);
        let value = sample_attr_value(spec, &attr, rng).unwrap_or_else(|| "unknown".to_string());
        Some(serde_json::json!({"op": "update", "id": id, "attribute": attr, "value": value}))
    } else {
        if *next_id == 0 {
            return None;
        }
        let id = rng.random_range(0..*next_id);
        Some(serde_json::json!({"op": "delete", "id": id}))
    }
}

/// Minimal CSV escaping for values that contain commas, quotes, or newlines.
fn csv_escape(v: &str) -> String {
    if v.contains([',', '"', '\n']) {
        format!("\"{}\"", v.replace('"', "\"\""))
    } else {
        v.to_string()
    }
}

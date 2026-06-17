//! Minimal DuckDB comparison runner for JimVD.
//!
//! Loads the CSV dataset produced by `jimvd gen_dataset`, replays the same
//! engine-agnostic operation log against an in-memory DuckDB database, measures
//! per-operation latency, and emits a metrics report (P50/P99 latency, total
//! time, row count).

use anyhow::{Context, Result};
use clap::Parser;
use duckdb::Connection;
use serde::Serialize;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::time::Instant;

#[derive(Parser, Debug)]
#[command(about = "Replay a JimVD operation log against DuckDB and report latency")]
struct Args {
    /// Directory containing employees.csv / oplog.jsonl / meta.json.
    #[arg(long, default_value = "data")]
    data_dir: String,

    /// Output JSON report path.
    #[arg(long, default_value = "duckdb_report.json")]
    out: String,
}

#[derive(Debug, Serialize)]
struct MetricsReport {
    engine: String,
    operations: usize,
    insert_ops: usize,
    update_ops: usize,
    delete_ops: usize,
    query_ops: usize,
    /// Total rows returned across all read queries.
    query_rows: u64,
    /// Rows remaining in the table after replay.
    final_row_count: u64,
    p50_latency_us: f64,
    p99_latency_us: f64,
    total_time_secs: f64,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let data_dir = PathBuf::from(&args.data_dir);

    let meta: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(data_dir.join("meta.json"))?)
            .context("reading meta.json")?;
    let columns: Vec<String> = meta["columns"]
        .as_array()
        .context("meta.columns missing")?
        .iter()
        .map(|v| v.as_str().unwrap_or_default().to_string())
        .collect();

    let conn = Connection::open_in_memory()?;
    create_and_load(&conn, &data_dir, &columns)?;

    let oplog = fs::File::open(data_dir.join("oplog.jsonl")).context("opening oplog.jsonl")?;
    let reader = BufReader::new(oplog);

    let mut latencies_us: Vec<f64> = Vec::new();
    let mut insert_ops = 0usize;
    let mut update_ops = 0usize;
    let mut delete_ops = 0usize;
    let mut query_ops = 0usize;
    let mut query_rows: u64 = 0;

    let run_start = Instant::now();
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let op: serde_json::Value = serde_json::from_str(&line)?;
        let kind = op["op"].as_str().unwrap_or("");

        let start = Instant::now();
        match kind {
            "query_eq" => {
                let rows = exec_query_eq(&conn, &op)?;
                query_rows += rows;
                query_ops += 1;
            }
            "query_and" => {
                let rows = exec_query_and(&conn, &op)?;
                query_rows += rows;
                query_ops += 1;
            }
            "query_or" => {
                let rows = exec_query_or(&conn, &op)?;
                query_rows += rows;
                query_ops += 1;
            }
            "insert" => {
                exec_insert(&conn, &op, &columns)?;
                insert_ops += 1;
            }
            "update" => {
                exec_update(&conn, &op)?;
                update_ops += 1;
            }
            "delete" => {
                exec_delete(&conn, &op)?;
                delete_ops += 1;
            }
            other => eprintln!("[warn] unknown op kind: {}", other),
        }
        latencies_us.push(start.elapsed().as_secs_f64() * 1e6);
    }
    let total_time_secs = run_start.elapsed().as_secs_f64();

    let final_row_count: u64 =
        conn.query_row("SELECT COUNT(*) FROM employees", [], |r| r.get(0))?;

    let report = MetricsReport {
        engine: "duckdb".to_string(),
        operations: latencies_us.len(),
        insert_ops,
        update_ops,
        delete_ops,
        query_ops,
        query_rows,
        final_row_count,
        p50_latency_us: percentile(&mut latencies_us.clone(), 0.50),
        p99_latency_us: percentile(&mut latencies_us.clone(), 0.99),
        total_time_secs,
    };

    println!("\n╔══════════════════════════════════════════╗");
    println!("║           DuckDB Runner Report           ║");
    println!("╠══════════════════════════════════════════╣");
    println!("║  Operations          {:>15} ║", report.operations);
    println!("║  Query ops           {:>15} ║", report.query_ops);
    println!("║  Insert ops          {:>15} ║", report.insert_ops);
    println!("║  Update ops          {:>15} ║", report.update_ops);
    println!("║  Delete ops          {:>15} ║", report.delete_ops);
    println!("║  Query rows returned {:>15} ║", report.query_rows);
    println!("║  Final row count     {:>15} ║", report.final_row_count);
    println!("║  P50 latency (µs)    {:>15.2} ║", report.p50_latency_us);
    println!("║  P99 latency (µs)    {:>15.2} ║", report.p99_latency_us);
    println!("║  Total time (s)      {:>15.4} ║", report.total_time_secs);
    println!("╚══════════════════════════════════════════╝");

    fs::write(&args.out, serde_json::to_string_pretty(&report)?)?;
    println!("Report written to {}", args.out);

    Ok(())
}

fn create_and_load(conn: &Connection, data_dir: &PathBuf, columns: &[String]) -> Result<()> {
    let cols_ddl: String = columns
        .iter()
        .map(|c| format!("\"{}\" VARCHAR", c))
        .collect::<Vec<_>>()
        .join(", ");
    conn.execute_batch(&format!(
        "CREATE TABLE employees (id INTEGER, {});",
        cols_ddl
    ))?;

    let csv_path = data_dir.join("employees.csv");
    conn.execute_batch(&format!(
        "COPY employees FROM '{}' (FORMAT CSV, HEADER, NULLSTR '');",
        csv_path.display()
    ))?;
    Ok(())
}

fn sql_str(v: &str) -> String {
    format!("'{}'", v.replace('\'', "''"))
}

fn exec_query_eq(conn: &Connection, op: &serde_json::Value) -> Result<u64> {
    let attr = op["attribute"].as_str().unwrap_or_default();
    let value = op["value"].as_str().unwrap_or_default();
    let sql = format!(
        "SELECT COUNT(*) FROM employees WHERE \"{}\" = {}",
        attr,
        sql_str(value)
    );
    Ok(conn.query_row(&sql, [], |r| r.get::<_, i64>(0))? as u64)
}

/// Conjunctive filter implemented as a chain of JOINs over per-predicate id
/// subqueries, mirroring JimVD's extent intersection.
fn exec_query_and(conn: &Connection, op: &serde_json::Value) -> Result<u64> {
    let filters = op["filters"].as_array().cloned().unwrap_or_default();
    if filters.is_empty() {
        return Ok(0);
    }

    let mut from = String::new();
    for (i, f) in filters.iter().enumerate() {
        let attr = f["attribute"].as_str().unwrap_or_default();
        let value = f["value"].as_str().unwrap_or_default();
        let sub = format!(
            "(SELECT id FROM employees WHERE \"{}\" = {}) t{}",
            attr,
            sql_str(value),
            i
        );
        if i == 0 {
            from.push_str(&sub);
        } else {
            from.push_str(&format!(" JOIN {} USING (id)", sub));
        }
    }
    let sql = format!("SELECT COUNT(*) FROM {}", from);
    Ok(conn.query_row(&sql, [], |r| r.get::<_, i64>(0))? as u64)
}

fn exec_query_or(conn: &Connection, op: &serde_json::Value) -> Result<u64> {
    let attr = op["attribute"].as_str().unwrap_or_default();
    let values = op["values"].as_array().cloned().unwrap_or_default();
    if values.is_empty() {
        return Ok(0);
    }
    let in_list = values
        .iter()
        .map(|v| sql_str(v.as_str().unwrap_or_default()))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT COUNT(*) FROM employees WHERE \"{}\" IN ({})",
        attr, in_list
    );
    Ok(conn.query_row(&sql, [], |r| r.get::<_, i64>(0))? as u64)
}

fn exec_insert(conn: &Connection, op: &serde_json::Value, columns: &[String]) -> Result<()> {
    let id = op["id"].as_u64().unwrap_or(0);
    let values = &op["values"];

    let mut col_names = vec!["id".to_string()];
    let mut col_vals = vec![id.to_string()];
    for c in columns {
        if let Some(v) = values.get(c).and_then(|x| x.as_str()) {
            col_names.push(format!("\"{}\"", c));
            col_vals.push(sql_str(v));
        }
    }
    let sql = format!(
        "INSERT INTO employees ({}) VALUES ({})",
        col_names.join(", "),
        col_vals.join(", ")
    );
    conn.execute_batch(&sql)?;
    Ok(())
}

fn exec_update(conn: &Connection, op: &serde_json::Value) -> Result<()> {
    let id = op["id"].as_u64().unwrap_or(0);
    let attr = op["attribute"].as_str().unwrap_or_default();
    let value = op["value"].as_str().unwrap_or_default();
    let sql = format!(
        "UPDATE employees SET \"{}\" = {} WHERE id = {}",
        attr,
        sql_str(value),
        id
    );
    conn.execute_batch(&sql)?;
    Ok(())
}

fn exec_delete(conn: &Connection, op: &serde_json::Value) -> Result<()> {
    let id = op["id"].as_u64().unwrap_or(0);
    conn.execute_batch(&format!("DELETE FROM employees WHERE id = {}", id))?;
    Ok(())
}

/// Nearest-rank percentile over a latency sample (q in [0,1]).
fn percentile(samples: &mut [f64], q: f64) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let rank = (q * (samples.len() as f64 - 1.0)).round() as usize;
    samples[rank.min(samples.len() - 1)]
}

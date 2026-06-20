use anyhow::Result;
use jimvd::{
    cover::GreedyCover,
    graph::{FactorGraph, MultiTableGraph},
    metrics::Metrics,
    types::{Delta, DeltaType, Query, QueryFilter},
    workload::{AttributeDef, AttributeSpec, WorkloadConfig},
};
use rand::{Rng, RngExt, SeedableRng};
use std::collections::HashMap;
use std::fs;
use std::sync::atomic::Ordering;
use std::time::Instant;

fn main() -> Result<()> {
    let config_path = std::env::args().nth(1)
        .unwrap_or_else(|| "config/join_workload.json".to_string());
    let config_str = fs::read_to_string(&config_path)?;
    let config: WorkloadConfig = serde_json::from_str(&config_str)?;

    println!("=== Join Benchmark (direct): {} ===", config.workload_name);
    println!("{}\n", config.description);

    let mut m_graph = MultiTableGraph::new();
    let metrics = Metrics::new();

    // ---- Initialise tables (sorted for deterministic output) ----
    let mut table_names: Vec<&String> = config.tables.keys().collect();
    table_names.sort();
    let mut next_oid: u32 = 0;

    for table_name in &table_names {
        let spec = &config.tables[*table_name];
        let mut graph = FactorGraph::new();
        graph.materialization_threshold = config.adaptation.materialization_threshold;
        let mut rng = rand::rngs::StdRng::seed_from_u64(42);

        let mut raw_data: Vec<(u32, HashMap<String, String>)> = Vec::new();
        for _ in 0..spec.initial_objects {
            let oid = next_oid;
            next_oid += 1;
            let mut props = HashMap::new();
            for (attr, def) in &spec.attributes {
                let vals = attr_def_values(def);
                if !vals.is_empty() {
                    props.insert(attr.clone(), vals[rng.random_range(0..vals.len())].clone());
                }
            }
            raw_data.push((oid, props));
        }

        // Only factorise enumerated attributes (same logic as BenchmarkRunner).
        let factorize_set: std::collections::HashSet<String> =
            match &spec.factorize_attributes {
                Some(list) => list.iter().cloned().collect(),
                None => spec.attributes.iter()
                    .filter(|(_, def)| !matches!(def, AttributeDef::Extended(AttributeSpec::Continuous { .. })))
                    .map(|(k, _)| k.clone())
                    .collect(),
            };

        let seed_for_cover: Vec<(u32, HashMap<String, String>)> = raw_data.iter()
            .map(|(oid, props)| {
                let factorized = props.iter()
                    .filter(|(k, _)| factorize_set.contains(*k))
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                (*oid, factorized)
            })
            .collect();

        let mut cover = GreedyCover::new(seed_for_cover);
        let factors = cover.build_factors();
        println!("[Init:{}] {} objects → {} structural factors", table_name, spec.initial_objects, factors.len());
        for f in factors { graph.add_factor(f); }
        // Non-factorized props stored in overflow; live_ids populated by add_factor
        for (oid, props) in &raw_data {
            let non_fact: HashMap<String, String> = props.iter()
                .filter(|(k, _)| !graph.bpi.keys().any(|a| a.starts_with(&format!("{}=", k))))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            if !non_fact.is_empty() { graph.overflow.insert(*oid, non_fact); }
        }
        m_graph.add_table(table_name.to_string(), graph);
    }

    // ---- Build query pool from raw JSON (avoids extending QueryTemplate) ----
    let mut rng = rand::rngs::StdRng::seed_from_u64(99);
    let raw: serde_json::Value = serde_json::from_str(&config_str)?;
    let raw_pool = raw["query_mix"].as_array().unwrap();
    let mut pool: Vec<Query> = Vec::new();

    for q in raw_pool {
        let qtype  = q["type"].as_str().unwrap();
        let weight = q["weight"].as_u64().unwrap_or(1) as usize;
        for _ in 0..weight {
            match qtype {
                "eq" => {
                    let table = q["table"].as_str().unwrap_or("employees").to_string();
                    let attr  = q["attribute"].as_str().unwrap().to_string();
                    let vals: Vec<String> = q["values"].as_array().unwrap().iter()
                        .map(|v| v.as_str().unwrap().to_string()).collect();
                    let val = vals[rng.random_range(0..vals.len())].clone();
                    pool.push(Query::Filter {
                        table,
                        filter: QueryFilter::Eq { attribute: attr, value: val },
                    });
                }
                "and" => {
                    let table = q["table"].as_str().unwrap_or("employees").to_string();
                    let attrs: Vec<String> = q["attributes"].as_array().unwrap().iter()
                        .map(|v| v.as_str().unwrap().to_string()).collect();
                    let spec = &config.tables[&table];
                    let mut filters = Vec::new();
                    for attr in &attrs {
                        if let Some(def) = spec.attributes.get(attr) {
                            let vals = attr_def_values(def);
                            if !vals.is_empty() {
                                let val = vals[rng.random_range(0..vals.len())].clone();
                                filters.push(QueryFilter::Eq { attribute: attr.clone(), value: val });
                            }
                        }
                    }
                    pool.push(Query::Filter { table, filter: QueryFilter::And(filters) });
                }
                "join" => {
                    let left_table  = q["left_table"].as_str().unwrap().to_string();
                    let right_table = q["right_table"].as_str().unwrap().to_string();
                    let join_attr   = q["join_attribute"].as_str().unwrap().to_string();
                    let lf = build_filters(q["left_filters"].as_array().unwrap(),  &config, &left_table,  &mut rng);
                    let rf = build_filters(q["right_filters"].as_array().unwrap(), &config, &right_table, &mut rng);
                    pool.push(Query::Join {
                        left_table, right_table,
                        join_attribute: join_attr,
                        left_filters:  lf,
                        right_filters: rf,
                    });
                }
                _ => {}
            }
        }
    }

    println!("[Pool] {} query instances\n", pool.len());

    // ---- Run workload ----
    let total       = config.run_options.total_operations;
    let warmup      = config.run_options.warmup_ops;
    let interval    = config.run_options.metrics_interval_ops;
    let wm          = &config.write_mix;
    let write_total = wm.insert_rate + wm.update_rate + wm.delete_rate;
    let mut next_snapshot = warmup + interval;
    let mut next_oid_w    = next_oid;
    let start = Instant::now();

    for op in 0..total {
        if op == warmup {
            println!("[Warmup done] Beginning measured workload…\n");
        }

        let roll: f64 = rng.random();
        if roll < write_total {
            let wroll: f64 = rng.random::<f64>() * write_total;
            let tbl = &wm.table;

            if wroll < wm.insert_rate {
                let new_id = next_oid_w;
                next_oid_w += 1;
                let spec = &config.tables[tbl];
                let mut kv = serde_json::Map::new();
                kv.insert("id".into(), serde_json::Value::from(new_id));
                for attr in &wm.attributes {
                    if let Some(def) = spec.attributes.get(attr) {
                        let vals = attr_def_values(def);
                        if !vals.is_empty() {
                            let v = vals[rng.random_range(0..vals.len())].clone();
                            kv.insert(attr.clone(), serde_json::Value::String(v));
                        }
                    }
                }
                let delta = make_delta(DeltaType::Insert, tbl, kv);
                m_graph.apply_delta(tbl, &delta, &metrics);

            } else if wroll < wm.insert_rate + wm.update_rate {
                let ids: Vec<u32> = m_graph.table(tbl).live_ids.iter().cloned().collect();
                if ids.is_empty() { continue; }
                let target = ids[rng.random_range(0..ids.len())];
                let existing: HashMap<String, String> =
                    m_graph.table(tbl).reconstruct_object(target);
                let spec = &config.tables[tbl];
                let attr = &wm.attributes[rng.random_range(0..wm.attributes.len())];
                if let Some(def) = spec.attributes.get(attr) {
                    let vals = attr_def_values(def);
                    if vals.is_empty() { continue; }
                    let mut new_props = existing;
                    new_props.insert(attr.clone(), vals[rng.random_range(0..vals.len())].clone());
                    let mut kv = serde_json::Map::new();
                    kv.insert("id".into(), serde_json::Value::from(target));
                    for (k, v) in &new_props {
                        kv.insert(k.clone(), serde_json::Value::String(v.clone()));
                    }
                    let delta = make_delta(DeltaType::Update, tbl, kv);
                    m_graph.apply_delta(tbl, &delta, &metrics);
                }
            } else {
                let ids: Vec<u32> = m_graph.table(tbl).live_ids.iter().cloned().collect();
                if ids.is_empty() { continue; }
                let target = ids[rng.random_range(0..ids.len())];
                let mut kv = serde_json::Map::new();
                kv.insert("id".into(), serde_json::Value::from(target));
                let delta = make_delta(DeltaType::Delete, tbl, kv);
                m_graph.apply_delta(tbl, &delta, &metrics);
            }
        } else {
            let q = pool[rng.random_range(0..pool.len())].clone();
            match q {
                Query::Filter { table, filter } => {
                    m_graph.query_table(&table, &filter, &metrics);
                }
                Query::Join { left_table, right_table, join_attribute, left_filters, right_filters } => {
                    m_graph.factor_join(
                        &left_table, &right_table, &join_attribute,
                        &left_filters, &right_filters, &metrics,
                    );
                }
            }
        }

        if op >= warmup && op == next_snapshot {
            let s_count: usize = m_graph.tables.values()
                .map(|t| t.factors.values().filter(|f|  f.is_structural).count()).sum();
            let o_count: usize = m_graph.tables.values()
                .map(|t| t.factors.values().filter(|f| !f.is_structural).count()).sum();
            println!(
                "[op {:>5}]  queries={:>5}  util={:>5.1}%  uaf={:.2}  S/O={}/{}",
                op,
                metrics.total_queries.load(Ordering::Relaxed),
                metrics.factor_utilization() * 100.0,
                metrics.uaf(),
                s_count, o_count,
            );
            next_snapshot += interval;
        }
    }

    let s_count: usize = m_graph.tables.values()
        .map(|t| t.factors.values().filter(|f|  f.is_structural).count()).sum();
    let o_count: usize = m_graph.tables.values()
        .map(|t| t.factors.values().filter(|f| !f.is_structural).count()).sum();

    println!("\n╔══════════════════════════════════════╗");
    println!("║      Join-Direct Benchmark Summary   ║");
    println!("╠══════════════════════════════════════╣");
    println!("║  Total queries       {:>15} ║", metrics.total_queries.load(Ordering::Relaxed));
    println!("║  Query factor ops    {:>15} ║", metrics.query_factor_ops.load(Ordering::Relaxed));
    println!("║  Row ops             {:>15} ║", metrics.row_ops.load(Ordering::Relaxed));
    println!("║  Factor utilization  {:>14.1}% ║", metrics.factor_utilization() * 100.0);
    println!("║  Update Ampl. Factor {:>15.2} ║", metrics.uaf());
    println!("║  Structural factors  {:>15} ║", s_count);
    println!("║  Operational factors {:>15} ║", o_count);
    println!("║  Elapsed             {:>12?}   ║", start.elapsed());
    println!("╚══════════════════════════════════════╝");

    Ok(())
}

fn attr_def_values(def: &AttributeDef) -> &[String] {
    match def {
        AttributeDef::Simple(vals) => vals,
        AttributeDef::Extended(AttributeSpec::Categorical { values, .. }) => values,
        AttributeDef::Extended(AttributeSpec::Continuous { .. }) => &[],
    }
}

fn make_delta(dtype: DeltaType, table: &str, kv: serde_json::Map<String, serde_json::Value>) -> Delta {
    Delta {
        delta_id: 0, db_id: 0,
        base_version: "v1".into(), sequence: 0,
        delta_type: dtype,
        table_name: table.to_string(),
        codomain_ids: vec![], contact_ids: vec![],
        operation_details: serde_json::Value::Object(kv),
    }
}

fn build_filters(
    filters_json: &[serde_json::Value],
    config: &WorkloadConfig,
    table: &str,
    rng: &mut impl Rng,
) -> Vec<QueryFilter> {
    let mut filters = Vec::new();
    for f in filters_json {
        match f["type"].as_str().unwrap_or("") {
            "eq" => {
                let attr = f["attribute"].as_str().unwrap().to_string();
                let vals: Vec<String> = f["values"].as_array().unwrap().iter()
                    .map(|v| v.as_str().unwrap().to_string()).collect();
                let val = if !vals.is_empty() {
                    vals[rng.random_range(0..vals.len())].clone()
                } else if let Some(def) = config.tables.get(table)
                    .and_then(|t| t.attributes.get(&attr))
                {
                    let cfg_vals = attr_def_values(def);
                    if cfg_vals.is_empty() { continue; }
                    cfg_vals[rng.random_range(0..cfg_vals.len())].clone()
                } else {
                    continue;
                };
                filters.push(QueryFilter::Eq { attribute: attr, value: val });
            }
            _ => {}
        }
    }
    filters
}

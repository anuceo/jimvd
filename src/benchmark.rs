use crate::cover::GreedyCover;
use crate::graph::{FactorGraph, MultiTableGraph};
use crate::metrics::Metrics;
use crate::types::{Delta, DeltaType, MetricsReport, QueryFilter};
use crate::workload::{AttributeDef, AttributeSpec, QueryTemplate, WorkloadConfig};
use rand::{Rng, RngExt, SeedableRng};
use rand::rngs::StdRng;
use std::collections::HashMap;

// Internal query representation built from a QueryTemplate.
enum Query {
    Filter {
        table: String,
        filter: QueryFilter,
    },
    Join {
        left_table: String,
        right_table: String,
        join_attr: String,
        left_filters: Vec<QueryFilter>,
        right_filters: Vec<QueryFilter>,
    },
}

pub struct BenchmarkRunner {
    pub config: WorkloadConfig,
    multi: MultiTableGraph,
    pub metrics: Metrics,
    next_object_id: u32,
    next_delta_id: i64,
    next_eviction_tick: u64,
    /// Per-interval snapshots collected during run(). Cleared by change_workload().
    pub snapshots: Vec<(usize, MetricsReport)>,
    /// Per-delta propagation fanout (nodes touched per write operation).
    pub fanout_log: Vec<u64>,
    /// Monotonically increasing across multiple run() calls.
    pub total_ops_executed: usize,
    /// Attached to every snapshot emitted by run().
    pub current_phase_name: String,
    pub cover_time_ms: u64,
}

impl BenchmarkRunner {
    pub fn new(config: WorkloadConfig) -> Self {
        let next_eviction_tick = config.adaptation.eviction_ticks;
        BenchmarkRunner {
            config,
            multi: MultiTableGraph::new(),
            metrics: Metrics::new(),
            next_object_id: 0,
            next_delta_id: 1,
            next_eviction_tick,
            snapshots: Vec::new(),
            fanout_log: Vec::new(),
            total_ops_executed: 0,
            current_phase_name: String::new(),
            cover_time_ms: 0,
        }
    }

    /// Swap the workload config without resetting the graph or metrics.
    /// Updates adaptation parameters on all live tables.
    pub fn change_workload(&mut self, new_config: WorkloadConfig) {
        let threshold = new_config.adaptation.materialization_threshold;
        for graph in self.multi.tables.values_mut() {
            graph.materialization_threshold = threshold;
        }
        self.next_eviction_tick = self.multi.max_tick() + new_config.adaptation.eviction_ticks;
        self.config = new_config;
        self.snapshots.clear();
    }

    pub fn initialize(&mut self) {
        let mut rng: StdRng = if self.config.rng_seed == 0 {
            rand::make_rng()
        } else {
            StdRng::seed_from_u64(self.config.rng_seed)
        };
        println!("[Init] rng_seed={}", self.config.rng_seed);
        // Collect table names first to avoid borrow conflicts.
        let table_names: Vec<String> = self.config.tables.keys().cloned().collect();

        for table_name in &table_names {
            let spec = &self.config.tables[table_name];

            let factorize_set: std::collections::HashSet<String> =
                match &spec.factorize_attributes {
                    Some(list) => list.iter().cloned().collect(),
                    None => spec.attributes.iter()
                        .filter(|(_, def)| matches!(def, AttributeDef::Simple(_) | AttributeDef::Extended(AttributeSpec::Categorical { .. })))
                        .map(|(k, _)| k.clone())
                        .collect(),
                };

            let mut graph = FactorGraph::new();
            graph.materialization_threshold = self.config.adaptation.materialization_threshold;

            let mut seed_objects: Vec<(u32, HashMap<String, String>)> = Vec::new();

            for _ in 0..spec.initial_objects {
                let oid = self.next_object_id;
                self.next_object_id += 1;

                let mut props: HashMap<String, String> = HashMap::new();
                for (attr, attr_def) in &spec.attributes {
                    let val = sample_attr_value(attr_def, &mut rng);
                    if val != "__NULL__" {
                        props.insert(attr.clone(), val);
                    }
                }

                let factorized: HashMap<String, String> = props.iter()
                    .filter(|(k, _)| factorize_set.contains(k.as_str()))
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                // Non-factorized attrs stored in overflow after factors are built
                let non_factorized: HashMap<String, String> = props.into_iter()
                    .filter(|(k, _)| !factorize_set.contains(k.as_str()))
                    .collect();
                if !non_factorized.is_empty() {
                    graph.overflow.insert(oid, non_factorized);
                }
                graph.live_ids.insert(oid);
                seed_objects.push((oid, factorized));
            }

            let n = seed_objects.len();
            let t0 = std::time::Instant::now();
            let mut cover = GreedyCover::new(seed_objects);
            let factors = cover.build_factors();
            let cover_ms = t0.elapsed().as_millis() as u64;
            self.cover_time_ms += cover_ms;
            let f = factors.len();
            println!("[Init:{}] Seeded {} objects → {} structural factors (cover took {}ms)", table_name, n, f, cover_ms);
            for factor in factors {
                graph.add_factor(factor);
            }
            self.multi.add_table(table_name.clone(), graph);
        }
    }

    pub fn run(&mut self) {
        let total    = self.config.run_options.total_operations;
        let interval = self.config.run_options.metrics_interval_ops;
        let warmup   = self.config.run_options.warmup_ops;
        let write_rate = self.config.write_mix.insert_rate
            + self.config.write_mix.update_rate
            + self.config.write_mix.delete_rate;
        let query_weight: u32 = self.config.query_mix.iter().map(|t| t.weight()).sum();

        let mut rng: StdRng = if self.config.rng_seed == 0 {
            rand::make_rng()
        } else {
            StdRng::seed_from_u64(self.config.rng_seed)
        };

        for i in 0..total {
            let op_idx = self.total_ops_executed + i;

            if i == warmup {
                println!("[Warmup done] Beginning measured workload…\n");
            }

            if rng.random::<f64>() < write_rate {
                self.do_write(&mut rng);
            } else {
                self.do_read(&mut rng, query_weight);
            }

            // Evict stale operational factors on the configured tick cadence.
            if self.multi.max_tick() >= self.next_eviction_tick {
                self.multi.evict_all(self.config.adaptation.eviction_ticks);
                self.next_eviction_tick += self.config.adaptation.eviction_ticks;
            }

            if i > 0 && i % interval == 0 {
                let mut r = self.multi.gather_metrics(&self.metrics);
                r.phase_name = self.current_phase_name.clone();
                println!(
                    "[{:<3} op {:>6}]  queries={:>5}  util={:>5.1}%  uaf={:.2}  S/O={}/{}",
                    r.phase_name,
                    op_idx,
                    r.total_queries,
                    r.factor_utilization * 100.0,
                    r.uaf,
                    r.structural_factor_count,
                    r.operational_factor_count,
                );
                self.snapshots.push((op_idx, r));
            }
        }

        self.total_ops_executed += total;
    }

    fn do_read(&mut self, rng: &mut impl Rng, total_weight: u32) {
        if total_weight == 0 { return; }
        let pick: u32 = rng.random_range(0..total_weight);
        // select_query borrows &self and returns an owned Query — borrow ends before the mut calls.
        let query = self.select_query(pick, rng);
        if let Some(q) = query {
            let m = &self.metrics;
            match q {
                Query::Filter { table, filter } => {
                    self.multi.table_mut(&table).query_with_fallback(&filter, m);
                }
                Query::Join { left_table, right_table, join_attr, left_filters, right_filters } => {
                    self.multi.factor_join(
                        &left_table, &right_table, &join_attr,
                        &left_filters, &right_filters, m,
                    );
                }
            }
        }
    }

    fn select_query(&self, pick: u32, rng: &mut impl Rng) -> Option<Query> {
        let mut cumulative = 0u32;
        for template in &self.config.query_mix {
            cumulative += template.weight();
            if pick < cumulative {
                return Some(self.build_query(template, rng));
            }
        }
        None
    }

    fn build_query(&self, template: &QueryTemplate, rng: &mut impl Rng) -> Query {
        match template {
            QueryTemplate::Eq { attribute, values, table, hot_values, .. } => {
                let v = if !hot_values.is_empty() {
                    weighted_sample_hot(hot_values, rng)
                } else if values.is_empty() {
                    self.sample_value(attribute, rng)
                } else {
                    values[rng.random_range(0..values.len())].clone()
                };
                Query::Filter {
                    table: table.clone(),
                    filter: QueryFilter::Eq { attribute: attribute.clone(), value: v },
                }
            }
            QueryTemplate::And { attributes, table, .. } => {
                let subs = attributes.iter()
                    .map(|a| {
                        let v = self.sample_value(a, rng);
                        QueryFilter::Eq { attribute: a.clone(), value: v }
                    })
                    .collect();
                Query::Filter { table: table.clone(), filter: QueryFilter::And(subs) }
            }
            QueryTemplate::Or { attributes, values, table, .. } => {
                let attr = attributes[0].clone();
                let subs = values.iter()
                    .map(|v| QueryFilter::Eq { attribute: attr.clone(), value: v.clone() })
                    .collect();
                Query::Filter { table: table.clone(), filter: QueryFilter::Or(subs) }
            }
            QueryTemplate::Join { left_table, right_table, join_attribute, left_filters, right_filters, .. } => {
                let lf: Vec<QueryFilter> = left_filters.iter()
                    .map(|t| self.build_filter_only(t, rng))
                    .collect();
                let rf: Vec<QueryFilter> = right_filters.iter()
                    .map(|t| self.build_filter_only(t, rng))
                    .collect();
                Query::Join {
                    left_table:   left_table.clone(),
                    right_table:  right_table.clone(),
                    join_attr:    join_attribute.clone(),
                    left_filters: lf,
                    right_filters: rf,
                }
            }
        }
    }

    /// Build a QueryFilter from a non-join template (used for nested left/right filters).
    fn build_filter_only(&self, template: &QueryTemplate, rng: &mut impl Rng) -> QueryFilter {
        match template {
            QueryTemplate::Eq { attribute, values, hot_values, .. } => {
                let v = if !hot_values.is_empty() {
                    weighted_sample_hot(hot_values, rng)
                } else if values.is_empty() {
                    self.sample_value(attribute, rng)
                } else {
                    values[rng.random_range(0..values.len())].clone()
                };
                QueryFilter::Eq { attribute: attribute.clone(), value: v }
            }
            QueryTemplate::And { attributes, .. } => {
                let subs = attributes.iter()
                    .map(|a| {
                        let v = self.sample_value(a, rng);
                        QueryFilter::Eq { attribute: a.clone(), value: v }
                    })
                    .collect();
                QueryFilter::And(subs)
            }
            QueryTemplate::Or { attributes, values, .. } => {
                let attr = attributes[0].clone();
                let subs = values.iter()
                    .map(|v| QueryFilter::Eq { attribute: attr.clone(), value: v.clone() })
                    .collect();
                QueryFilter::Or(subs)
            }
            QueryTemplate::Join { .. } => panic!("nested Join templates are not supported"),
        }
    }

    fn sample_value(&self, attr: &str, rng: &mut impl Rng) -> String {
        for spec in self.config.tables.values() {
            if let Some(def) = spec.attributes.get(attr) {
                let v = sample_attr_value(def, rng);
                if v != "__NULL__" { return v; }
            }
        }
        // Sample from known values in the factor space (BPI keys)
        let prefix = format!("{}=", attr);
        let vals: Vec<String> = self.multi.tables.values()
            .flat_map(|g| g.bpi.keys())
            .filter(|a| a.starts_with(&prefix))
            .filter_map(|a| a.splitn(2, '=').nth(1).map(|v| v.to_string()))
            .collect();
        if !vals.is_empty() {
            return vals[rng.random_range(0..vals.len())].clone();
        }
        "unknown".to_string()
    }

    fn do_write(&mut self, rng: &mut impl Rng) {
        let ins = self.config.write_mix.insert_rate;
        let upd = self.config.write_mix.update_rate;
        let del = self.config.write_mix.delete_rate;
        let total = ins + upd + del;
        if total == 0.0 { return; }

        let table_name = self.config.write_mix.table.clone();
        let roll: f64  = rng.random::<f64>() * total;

        let (dtype, obj_id) = if roll < ins {
            let id = self.next_object_id;
            self.next_object_id += 1;
            (DeltaType::Insert, id)
        } else if roll < ins + upd {
            if self.next_object_id == 0 { return; }
            (DeltaType::Update, rng.random_range(0..self.next_object_id))
        } else {
            if self.next_object_id == 0 { return; }
            (DeltaType::Delete, rng.random_range(0..self.next_object_id))
        };

        let mut details = serde_json::Map::new();
        details.insert("id".to_string(), serde_json::Value::Number(serde_json::Number::from(obj_id)));

        match &dtype {
            DeltaType::Insert => {
                let attrs: Vec<String> = self.config.write_mix.attributes.clone();
                for attr in &attrs {
                    let v = self.sample_value(attr, rng);
                    details.insert(attr.clone(), serde_json::Value::String(v));
                }
            }
            DeltaType::Update => {
                let existing: HashMap<String, String> = self.multi
                    .tables.get(&table_name)
                    .map(|g| g.reconstruct_object(obj_id))
                    .unwrap_or_default();
                for (k, v) in existing {
                    details.insert(k, serde_json::Value::String(v));
                }
                let attrs = self.config.write_mix.attributes.clone();
                if !attrs.is_empty() {
                    let attr = if let Some(weights) = &self.config.write_mix.attribute_weights {
                        weighted_sample_attr(&attrs, weights, rng)
                    } else {
                        attrs[rng.random_range(0..attrs.len())].clone()
                    };
                    let v = self.sample_value(&attr, rng);
                    details.insert(attr, serde_json::Value::String(v));
                }
            }
            DeltaType::Delete => {}
        }

        let delta = Delta {
            delta_id:   self.next_delta_id,
            db_id:      1,
            base_version: "v1".to_string(),
            sequence:   1,
            delta_type: dtype,
            table_name: table_name.clone(),
            codomain_ids: vec![],
            contact_ids:  vec![],
            operation_details: serde_json::Value::Object(details),
        };
        self.next_delta_id += 1;

        let before = self.metrics.write_propagation_nodes.load(std::sync::atomic::Ordering::Relaxed);
        let m = &self.metrics;
        self.multi.table_mut(&table_name).apply_delta(&delta, m);
        let after = self.metrics.write_propagation_nodes.load(std::sync::atomic::Ordering::Relaxed);
        self.fanout_log.push(after - before);
    }

    pub fn print_summary(&self) {
        let r = self.multi.gather_metrics(&self.metrics);
        println!("\n╔══════════════════════════════════════╗");
        println!("║         Benchmark Summary            ║");
        println!("╠══════════════════════════════════════╣");
        println!("║  Total queries       {:>15} ║", r.total_queries);
        println!("║  Query factor ops    {:>15} ║", r.query_factor_ops);
        println!("║  Write factor ops    {:>15} ║", r.write_factor_ops);
        println!("║  Row ops             {:>15} ║", r.row_ops);
        println!("║  Factor utilization  {:>14.1}% ║", r.factor_utilization * 100.0);
        println!("║  Query factor util   {:>14.1}% ║", r.query_factor_utilization * 100.0);
        println!("║  Update Ampl. Factor {:>15.2} ║", r.uaf);
        println!("║  Objects updated     {:>15} ║", r.objects_updated);
        println!("║  Write prop. nodes   {:>15} ║", r.write_propagation_nodes);
        println!("║  Ticks elapsed       {:>15} ║", r.current_tick);
        println!("║  Structural factors  {:>15} ║", r.structural_factor_count);
        println!("║  Operational factors {:>15} ║", r.operational_factor_count);
        println!("║  Evicted factors     {:>15} ║", r.evicted_factors.len());
        println!("╚══════════════════════════════════════╝");
    }
}

fn sample_attr_value(def: &crate::workload::AttributeDef, rng: &mut impl Rng) -> String {
    match def {
        AttributeDef::Simple(values) => {
            if values.is_empty() {
                rng.random_range(30_000u32..150_000u32).to_string()
            } else {
                values[rng.random_range(0..values.len())].clone()
            }
        }
        AttributeDef::Extended(spec) => match spec {
            AttributeSpec::Categorical { values, weights, null_probability } => {
                if rng.random::<f64>() < *null_probability { return "__NULL__".to_string(); }
                if let Some(w) = weights {
                    weighted_sample_values(values, w, rng)
                } else {
                    values[rng.random_range(0..values.len())].clone()
                }
            }
            AttributeSpec::Continuous { min, max, null_probability } => {
                if rng.random::<f64>() < *null_probability { return "__NULL__".to_string(); }
                rng.random_range(*min..=*max).to_string()
            }
        }
    }
}

fn weighted_sample_values(values: &[String], weights: &[f64], rng: &mut impl Rng) -> String {
    let total: f64 = weights.iter().sum();
    let pick = rng.random::<f64>() * total;
    let mut acc = 0.0;
    for (v, w) in values.iter().zip(weights.iter()) {
        acc += w;
        if pick <= acc { return v.clone(); }
    }
    values.last().cloned().unwrap_or_default()
}

fn weighted_sample_hot(hot_values: &[crate::workload::HotValue], rng: &mut impl Rng) -> String {
    let total: f64 = hot_values.iter().map(|h| h.weight).sum();
    let pick = rng.random::<f64>() * total;
    let mut acc = 0.0;
    for hv in hot_values {
        acc += hv.weight;
        if pick <= acc { return hv.value.clone(); }
    }
    hot_values.last().map(|h| h.value.clone()).unwrap_or_default()
}

fn weighted_sample_attr(attrs: &[String], weights: &HashMap<String, f64>, rng: &mut impl Rng) -> String {
    let total: f64 = attrs.iter().map(|a| weights.get(a).copied().unwrap_or(1.0)).sum();
    let pick = rng.random::<f64>() * total;
    let mut acc = 0.0;
    for a in attrs {
        acc += weights.get(a).copied().unwrap_or(1.0);
        if pick <= acc { return a.clone(); }
    }
    attrs.last().cloned().unwrap_or_default()
}

/// Invoke a Julia analysis script with the given snapshot file.
/// Gracefully skips if Julia is not installed rather than panicking.
pub fn run_julia_script(script: &str, snapshot_file: &str) {
    let result = std::process::Command::new("julia")
        .args(["--project=analysis", &format!("analysis/{}", script), snapshot_file])
        .status();

    match result {
        Ok(s) if s.success() => {}
        Ok(s) => eprintln!("[Julia] {} exited with {}", script, s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!(
                "[Julia] not installed — skipping {}. Run scripts/setup.sh to install.",
                script
            );
        }
        Err(e) => eprintln!("[Julia] failed to launch {}: {}", script, e),
    }
}

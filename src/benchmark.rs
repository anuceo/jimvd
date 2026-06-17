use crate::cover::GreedyCover;
use crate::graph::{FactorGraph, MultiTableGraph};
use crate::metrics::Metrics;
use crate::types::{Delta, DeltaType, MetricsReport, QueryFilter};
use crate::workload::{QueryTemplate, WorkloadConfig};
use rand::{Rng, RngExt};
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
    config: WorkloadConfig,
    multi: MultiTableGraph,
    metrics: Metrics,
    next_object_id: u32,
    next_delta_id: i64,
    next_eviction_tick: u64,
    /// Per-interval snapshots collected during run(). Cleared by change_workload().
    pub snapshots: Vec<(usize, MetricsReport)>,
    /// Per-delta propagation fanout (nodes touched per write operation).
    pub fanout_log: Vec<u64>,
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
        let mut rng = rand::rng();
        // Collect table names first to avoid borrow conflicts.
        let table_names: Vec<String> = self.config.tables.keys().cloned().collect();

        for table_name in &table_names {
            let spec = &self.config.tables[table_name];

            let factorize_set: std::collections::HashSet<String> =
                match &spec.factorize_attributes {
                    Some(list) => list.iter().cloned().collect(),
                    None => spec.attributes.iter()
                        .filter(|(_, v)| v.is_some())
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
                for (attr, maybe_values) in &spec.attributes {
                    let val = match maybe_values {
                        Some(values) if !values.is_empty() => {
                            values[rng.random_range(0..values.len())].clone()
                        }
                        _ => rng.random_range(30_000u32..150_000u32).to_string(),
                    };
                    props.insert(attr.clone(), val);
                }

                graph.objects.insert(oid, props.clone());

                let factorized: HashMap<String, String> = props.into_iter()
                    .filter(|(k, _)| factorize_set.contains(k))
                    .collect();
                seed_objects.push((oid, factorized));
            }

            let n = seed_objects.len();
            let mut cover = GreedyCover::new(seed_objects);
            let factors = cover.build_factors();
            let f = factors.len();
            for factor in factors {
                graph.add_factor(factor);
            }

            println!("[Init:{}] Seeded {} objects → {} structural factors", table_name, n, f);
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

        let mut rng = rand::rng();

        for i in 0..total {
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
                let r = self.multi.gather_metrics(&self.metrics);
                println!(
                    "[op {:>5}]  queries={:>5}  util={:>5.1}%  uaf={:.2}  S/O={}/{}",
                    i,
                    r.total_queries,
                    r.factor_utilization * 100.0,
                    r.uaf,
                    r.structural_factor_count,
                    r.operational_factor_count,
                );
                self.snapshots.push((i, r));
            }
        }
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
            QueryTemplate::Eq { attribute, values, table, .. } => {
                let v = if values.is_empty() {
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
            QueryTemplate::Eq { attribute, values, .. } => {
                let v = if values.is_empty() {
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
        // Try enumerated values from the config first.
        for spec in self.config.tables.values() {
            if let Some(Some(vals)) = spec.attributes.get(attr) {
                if !vals.is_empty() {
                    return vals[rng.random_range(0..vals.len())].clone();
                }
            }
        }
        // Non-enumerated attribute: sample an actual value from live objects across all tables.
        let vals: Vec<String> = self.multi.tables.values()
            .flat_map(|g| g.objects.values())
            .filter_map(|props| props.get(attr).cloned())
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
                    .and_then(|g| g.objects.get(&obj_id))
                    .cloned()
                    .unwrap_or_default();
                for (k, v) in existing {
                    details.insert(k, serde_json::Value::String(v));
                }
                let attrs = self.config.write_mix.attributes.clone();
                if !attrs.is_empty() {
                    let attr = attrs[rng.random_range(0..attrs.len())].clone();
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

        let before = self.metrics.nodes_touched_by_updates.load(std::sync::atomic::Ordering::Relaxed);
        let m = &self.metrics;
        self.multi.table_mut(&table_name).apply_delta(&delta, m);
        let after = self.metrics.nodes_touched_by_updates.load(std::sync::atomic::Ordering::Relaxed);
        self.fanout_log.push(after - before);
    }

    pub fn print_summary(&self) {
        let r = self.multi.gather_metrics(&self.metrics);
        println!("\n╔══════════════════════════════════════╗");
        println!("║         Benchmark Summary            ║");
        println!("╠══════════════════════════════════════╣");
        println!("║  Total queries       {:>15} ║", r.total_queries);
        println!("║  Factor ops          {:>15} ║", r.factor_ops);
        println!("║  Row ops             {:>15} ║", r.row_ops);
        println!("║  Factor utilization  {:>14.1}% ║", r.factor_utilization * 100.0);
        println!("║  Update Ampl. Factor {:>15.2} ║", r.uaf);
        println!("║  Objects updated     {:>15} ║", r.objects_updated);
        println!("║  Nodes touched       {:>15} ║", r.nodes_touched_by_updates);
        println!("║  Ticks elapsed       {:>15} ║", r.current_tick);
        println!("║  Structural factors  {:>15} ║", r.structural_factor_count);
        println!("║  Operational factors {:>15} ║", r.operational_factor_count);
        println!("║  Evicted factors     {:>15} ║", r.evicted_factors.len());
        println!("╚══════════════════════════════════════╝");
    }
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

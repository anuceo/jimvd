use crate::cover::GreedyCover;
use crate::graph::FactorGraph;
use crate::metrics::Metrics;
use crate::types::{Delta, DeltaType, MetricsReport, QueryFilter};
use crate::workload::{QueryTemplate, WorkloadConfig};
use rand::{Rng, RngExt};
use std::collections::HashMap;

pub struct BenchmarkRunner {
    config: WorkloadConfig,
    graph: FactorGraph,
    metrics: Metrics,
    next_object_id: u32,
    next_delta_id: i64,
    /// Per-interval snapshots collected during run(). Cleared by change_workload().
    pub snapshots: Vec<(usize, MetricsReport)>,
}

impl BenchmarkRunner {
    pub fn new(config: WorkloadConfig) -> Self {
        BenchmarkRunner {
            config,
            graph: FactorGraph::new(),
            metrics: Metrics::new(),
            next_object_id: 0,
            next_delta_id: 1,
            snapshots: Vec::new(),
        }
    }

    /// Swap the workload config without resetting the graph or metrics.
    /// Clears the snapshot list so the caller can collect per-phase snapshots separately.
    pub fn change_workload(&mut self, new_config: WorkloadConfig) {
        self.config = new_config;
        self.snapshots.clear();
    }

    pub fn initialize(&mut self) {
        let mut rng = rand::rng();
        let mut seed_objects: Vec<(u32, HashMap<String, String>)> = Vec::new();

        for (_name, spec) in &self.config.tables {
            for _ in 0..spec.initial_objects {
                let oid = self.next_object_id;
                self.next_object_id += 1;
                let mut props: HashMap<String, String> = HashMap::new();
                for (attr, values) in &spec.attributes {
                    let val = values[rng.random_range(0..values.len())].clone();
                    props.insert(attr.clone(), val);
                }
                self.graph.objects.insert(oid, props.clone());
                seed_objects.push((oid, props));
            }
        }

        let n = seed_objects.len();
        let mut cover = GreedyCover::new(seed_objects);
        let factors = cover.build_factors();
        let f = factors.len();
        for factor in factors {
            self.graph.add_factor(factor);
        }

        println!("[Init] Seeded {} objects → {} structural factors", n, f);
    }

    pub fn run(&mut self) {
        let total = self.config.run_options.total_operations;
        let interval = self.config.run_options.metrics_interval_ops;
        let warmup = self.config.run_options.warmup_ops;
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

            // Evict stale operational factors periodically
            if i > 0 && i % 1000 == 0 {
                self.graph.evict_operational_factors(500);
            }

            if i > 0 && i % interval == 0 {
                let r = self.graph.gather_metrics(&self.metrics);
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

    // Returns an owned filter so no borrow of self lingers into the mutable call below.
    fn do_read(&mut self, rng: &mut impl Rng, total_weight: u32) {
        if total_weight == 0 {
            return;
        }
        let pick: u32 = rng.random_range(0..total_weight);
        // select_filter takes &self → owned QueryFilter returned, borrow ends
        let filter = self.select_filter(pick, rng);
        if let Some(f) = filter {
            // explicit split borrow: &mut self.graph vs &self.metrics
            let m = &self.metrics;
            self.graph.query(&f, m);
        }
    }

    fn select_filter(&self, pick: u32, rng: &mut impl Rng) -> Option<QueryFilter> {
        let mut cumulative = 0u32;
        for template in &self.config.query_mix {
            cumulative += template.weight();
            if pick < cumulative {
                return Some(self.build_filter(template, rng));
            }
        }
        None
    }

    fn build_filter(&self, template: &QueryTemplate, rng: &mut impl Rng) -> QueryFilter {
        match template {
            QueryTemplate::Eq { attribute, values, .. } => {
                let v = values[rng.random_range(0..values.len())].clone();
                QueryFilter::Eq { attribute: attribute.clone(), value: v }
            }
            QueryTemplate::And { attributes, .. } => {
                let subs = attributes
                    .iter()
                    .map(|a| {
                        let v = self.sample_value(a, rng);
                        QueryFilter::Eq { attribute: a.clone(), value: v }
                    })
                    .collect();
                QueryFilter::And(subs)
            }
            QueryTemplate::Or { attributes, values, .. } => {
                let attr = attributes[0].clone();
                let subs = values
                    .iter()
                    .map(|v| QueryFilter::Eq { attribute: attr.clone(), value: v.clone() })
                    .collect();
                QueryFilter::Or(subs)
            }
        }
    }

    fn sample_value(&self, attr: &str, rng: &mut impl Rng) -> String {
        for spec in self.config.tables.values() {
            if let Some(vals) = spec.attributes.get(attr) {
                if !vals.is_empty() {
                    return vals[rng.random_range(0..vals.len())].clone();
                }
            }
        }
        "unknown".to_string()
    }

    fn do_write(&mut self, rng: &mut impl Rng) {
        let ins = self.config.write_mix.insert_rate;
        let upd = self.config.write_mix.update_rate;
        let del = self.config.write_mix.delete_rate;
        let total = ins + upd + del;
        if total == 0.0 {
            return;
        }
        let roll: f64 = rng.random::<f64>() * total;

        let (dtype, obj_id) = if roll < ins {
            let id = self.next_object_id;
            self.next_object_id += 1;
            (DeltaType::Insert, id)
        } else if roll < ins + upd {
            if self.next_object_id == 0 {
                return;
            }
            (DeltaType::Update, rng.random_range(0..self.next_object_id))
        } else {
            if self.next_object_id == 0 {
                return;
            }
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
                // Clone existing props so we release the borrow before apply_delta
                let existing: HashMap<String, String> =
                    self.graph.objects.get(&obj_id).cloned().unwrap_or_default();
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
            delta_id: self.next_delta_id,
            db_id: 1,
            base_version: "v1".to_string(),
            sequence: 1,
            delta_type: dtype,
            table_name: "employees".to_string(),
            codomain_ids: vec![],
            contact_ids: vec![],
            operation_details: serde_json::Value::Object(details),
        };
        self.next_delta_id += 1;

        // Split borrow: &mut self.graph, &self.metrics
        let m = &self.metrics;
        self.graph.apply_delta(&delta, m);
    }

    pub fn print_summary(&self) {
        let r = self.graph.gather_metrics(&self.metrics);
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
        .args(["--project=julia", &format!("julia/{}", script), snapshot_file])
        .status();

    match result {
        Ok(s) if s.success() => {}
        Ok(s) => eprintln!("[Julia] {} exited with {}", script, s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!(
                "[Julia] not installed — skipping {}. Run scripts/setup_julia.sh to install.",
                script
            );
        }
        Err(e) => eprintln!("[Julia] failed to launch {}: {}", script, e),
    }
}

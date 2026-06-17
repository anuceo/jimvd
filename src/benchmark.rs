use crate::cover::GreedyCover;
use crate::graph::FactorGraph;
use crate::metrics::Metrics;
use crate::types::{Delta, DeltaType, MetricsReport, QueryFilter};
use crate::workload::{HotValue, QueryTemplate, TableSpec, WorkloadConfig};
use rand::rngs::StdRng;
use rand::{Rng, RngExt, SeedableRng};
use std::collections::{HashMap, HashSet};
use std::time::Instant;

pub struct BenchmarkRunner {
    config: WorkloadConfig,
    graph: FactorGraph,
    metrics: Metrics,
    next_object_id: u32,
    next_delta_id: i64,
    /// Deterministic RNG seeded from `config.rng_seed`.
    rng: StdRng,
    /// The seed actually used (echoed into snapshot metadata).
    pub rng_seed: u64,
    /// Wall-clock seconds spent in the greedy covering step during `initialize`.
    pub cover_build_secs: f64,
    /// Number of structural factors produced by the covering step.
    pub cover_factor_count: usize,
    /// Per-interval snapshots collected during run(). Cleared by change_workload().
    pub snapshots: Vec<(usize, MetricsReport)>,
}

impl BenchmarkRunner {
    pub fn new(config: WorkloadConfig) -> Self {
        let rng_seed = config.rng_seed;
        BenchmarkRunner {
            config,
            graph: FactorGraph::new(),
            metrics: Metrics::new(),
            next_object_id: 0,
            next_delta_id: 1,
            rng: StdRng::seed_from_u64(rng_seed),
            rng_seed,
            cover_build_secs: 0.0,
            cover_factor_count: 0,
            snapshots: Vec::new(),
        }
    }

    /// Swap the workload config without resetting the graph or metrics.
    /// Clears the snapshot list so the caller can collect per-phase snapshots separately.
    /// The RNG stream is preserved so the run stays deterministic across phases.
    pub fn change_workload(&mut self, new_config: WorkloadConfig) {
        self.config = new_config;
        self.snapshots.clear();
    }

    pub fn initialize(&mut self) {
        println!("[Seed] rng_seed = {}", self.rng_seed);

        // Pull the RNG out so we can borrow &self.config immutably alongside it.
        let mut rng = std::mem::replace(&mut self.rng, StdRng::seed_from_u64(0));

        // Continuous attributes are deliberately not factorised; register them so
        // the query path knows to fall back to row scans.
        let mut continuous: HashSet<String> = HashSet::new();
        for spec in self.config.tables.values() {
            for attr in spec.continuous.keys() {
                continuous.insert(attr.clone());
            }
        }
        self.graph.continuous_attributes = continuous.clone();

        // `seed_objects` feeds the greedy cover and only contains categorical
        // (factorisable) atoms; continuous values live in `graph.objects` for
        // row reconstruction / row scans but never become factors.
        let mut seed_objects: Vec<(u32, HashMap<String, String>)> = Vec::new();

        for spec in self.config.tables.values() {
            for _ in 0..spec.initial_objects {
                let oid = self.next_object_id;
                self.next_object_id += 1;
                let props = generate_props(spec, &mut rng);

                let categorical: HashMap<String, String> = props
                    .iter()
                    .filter(|(k, _)| !continuous.contains(*k))
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();

                self.graph.objects.insert(oid, props);
                seed_objects.push((oid, categorical));
            }
        }

        let n = seed_objects.len();
        let cover_start = Instant::now();
        let mut cover = GreedyCover::new(seed_objects);
        let factors = cover.build_factors();
        self.cover_build_secs = cover_start.elapsed().as_secs_f64();
        let f = factors.len();
        self.cover_factor_count = f;
        for factor in factors {
            self.graph.add_factor(factor);
        }

        self.rng = rng;

        println!(
            "[Init] Seeded {} objects → {} structural factors in {:.4}s (GreedyCover::build_factors)",
            n, f, self.cover_build_secs
        );
    }

    pub fn run(&mut self) {
        let total = self.config.run_options.total_operations;
        let interval = self.config.run_options.metrics_interval_ops;
        let warmup = self.config.run_options.warmup_ops;
        let write_rate = self.config.write_mix.insert_rate
            + self.config.write_mix.update_rate
            + self.config.write_mix.delete_rate;
        let query_weight: u32 = self.config.query_mix.iter().map(|t| t.weight()).sum();

        let mut rng = std::mem::replace(&mut self.rng, StdRng::seed_from_u64(0));

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

        self.rng = rng;
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
            QueryTemplate::Eq { attribute, values, hot_values, .. } => {
                let v = sample_eq_value(values, hot_values, rng);
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
                    return sample_weighted(vals, spec.weights.get(attr), rng);
                }
            }
            if let Some(cs) = spec.continuous.get(attr) {
                return rng.random_range(cs.min..=cs.max).to_string();
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
                    let attr =
                        select_mutation_attribute(&attrs, &self.config.write_mix.attribute_weights, rng);
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

    /// Gather the current aggregated metrics report.
    pub fn report(&self) -> MetricsReport {
        self.graph.gather_metrics(&self.metrics)
    }

    pub fn print_summary(&self) {
        let r = self.graph.gather_metrics(&self.metrics);
        println!("\n╔════════════════════════════════════════════╗");
        println!("║              Benchmark Summary             ║");
        println!("╠════════════════════════════════════════════╣");
        println!("║  Total queries           {:>15} ║", r.total_queries);
        println!("║  Query factor ops        {:>15} ║", r.query_factor_ops);
        println!("║  Write factor ops        {:>15} ║", r.write_factor_ops);
        println!("║  Row ops                 {:>15} ║", r.row_ops);
        println!("║  Factor utilization      {:>14.1}% ║", r.factor_utilization * 100.0);
        println!("║  Query factor util.      {:>14.1}% ║", r.query_factor_utilization * 100.0);
        println!("║  Update Ampl. Factor     {:>15.2} ║", r.uaf);
        println!("║  Objects updated         {:>15} ║", r.objects_updated);
        println!("║  Write propagation nodes {:>15} ║", r.write_propagation_nodes);
        println!("║  Ticks elapsed           {:>15} ║", r.current_tick);
        println!("║  Structural factors      {:>15} ║", r.structural_factor_count);
        println!("║  Operational factors     {:>15} ║", r.operational_factor_count);
        println!("║  Evicted factors         {:>15} ║", r.evicted_factors.len());
        println!("║  Memory (bytes)          {:>15} ║", r.memory_bytes);
        println!("║  Storage est. (bytes)    {:>15} ║", r.storage_bytes);
        println!("╚════════════════════════════════════════════╝");
    }
}

/// Generate one object's property map from a table spec, honouring per-attribute
/// sampling weights, NULL probabilities (NULL = omitted attribute), and
/// continuous integer attributes.
pub fn generate_props(spec: &TableSpec, rng: &mut impl Rng) -> HashMap<String, String> {
    let mut props: HashMap<String, String> = HashMap::new();

    for (attr, values) in &spec.attributes {
        if values.is_empty() {
            continue;
        }
        if emit_null(spec, attr, rng) {
            continue;
        }
        let val = sample_weighted(values, spec.weights.get(attr), rng);
        props.insert(attr.clone(), val);
    }

    for (attr, cs) in &spec.continuous {
        if emit_null(spec, attr, rng) {
            continue;
        }
        let lo = cs.min.min(cs.max);
        let hi = cs.min.max(cs.max);
        props.insert(attr.clone(), rng.random_range(lo..=hi).to_string());
    }

    props
}

fn emit_null(spec: &TableSpec, attr: &str, rng: &mut impl Rng) -> bool {
    match spec.null_probability.get(attr) {
        Some(p) if *p > 0.0 => rng.random::<f64>() < *p,
        _ => false,
    }
}

/// Sample a value from `values` using optional parallel weights. Falls back to
/// uniform sampling when weights are absent, mis-sized, or sum to zero.
pub fn sample_weighted(values: &[String], weights: Option<&Vec<f64>>, rng: &mut impl Rng) -> String {
    if let Some(w) = weights {
        let total: f64 = w.iter().copied().filter(|x| *x > 0.0).sum();
        if w.len() == values.len() && total > 0.0 {
            let mut pick = rng.random::<f64>() * total;
            for (v, wt) in values.iter().zip(w.iter()) {
                pick -= wt.max(0.0);
                if pick <= 0.0 {
                    return v.clone();
                }
            }
            return values[values.len() - 1].clone();
        }
    }
    values[rng.random_range(0..values.len())].clone()
}

/// Sample a value for a single attribute from a table spec (categorical with
/// optional weights, or continuous integer range). Returns `None` if the spec
/// does not define the attribute.
pub fn sample_attr_value(spec: &TableSpec, attr: &str, rng: &mut impl Rng) -> Option<String> {
    if let Some(vals) = spec.attributes.get(attr) {
        if !vals.is_empty() {
            return Some(sample_weighted(vals, spec.weights.get(attr), rng));
        }
    }
    if let Some(cs) = spec.continuous.get(attr) {
        let lo = cs.min.min(cs.max);
        let hi = cs.min.max(cs.max);
        return Some(rng.random_range(lo..=hi).to_string());
    }
    None
}

/// Pick the value for an `Eq` query template. When `hot_values` is non-empty the
/// listed values are chosen with their stated probabilities; the remaining
/// probability mass falls back to a uniform pick over `values`.
pub fn sample_eq_value(values: &[String], hot_values: &[HotValue], rng: &mut impl Rng) -> String {
    if !hot_values.is_empty() {
        let hot_total: f64 = hot_values.iter().map(|h| h.probability.max(0.0)).sum();
        if hot_total > 0.0 && rng.random::<f64>() < hot_total {
            let mut pick = rng.random::<f64>() * hot_total;
            for h in hot_values {
                pick -= h.probability.max(0.0);
                if pick <= 0.0 {
                    return h.value.clone();
                }
            }
            return hot_values[hot_values.len() - 1].value.clone();
        }
    }
    values[rng.random_range(0..values.len())].clone()
}

/// Choose which attribute to mutate on an update. Uses `attribute_weights` when
/// provided (proportional selection); otherwise uniform over `attrs`.
pub fn select_mutation_attribute(
    attrs: &[String],
    attribute_weights: &HashMap<String, f64>,
    rng: &mut impl Rng,
) -> String {
    if !attribute_weights.is_empty() {
        let total: f64 = attrs
            .iter()
            .map(|a| attribute_weights.get(a).copied().unwrap_or(0.0).max(0.0))
            .sum();
        if total > 0.0 {
            let mut pick = rng.random::<f64>() * total;
            for a in attrs {
                pick -= attribute_weights.get(a).copied().unwrap_or(0.0).max(0.0);
                if pick <= 0.0 {
                    return a.clone();
                }
            }
        }
    }
    attrs[rng.random_range(0..attrs.len())].clone()
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

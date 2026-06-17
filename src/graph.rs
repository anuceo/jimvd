use crate::types::*;
use crate::metrics::Metrics;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::Ordering;

pub type BOI = HashMap<ObjectId, HashSet<u64>>;
pub type BPI = HashMap<PropertyAtom, HashSet<u64>>;

pub struct FactorGraph {
    pub graph: DependencyGraph,
    pub boi: BOI,
    pub bpi: BPI,
    pub factors: HashMap<u64, Factor>,
    pub objects: HashMap<ObjectId, HashMap<String, String>>,

    // adaptive materialisation
    pub conjunction_hits: HashMap<String, u64>,
    pub factor_last_access: HashMap<u64, u64>,
    pub current_tick: u64,

    // lifecycle tracking for metrics
    pub lifecycles: HashMap<u64, FactorLifecycle>,          // active factors
    pub completed_lifecycles: Vec<FactorLifecycle>,        // evicted factors
}

impl FactorGraph {
    pub fn new() -> Self {
        FactorGraph {
            graph: DependencyGraph::new(),
            boi: HashMap::new(),
            bpi: HashMap::new(),
            factors: HashMap::new(),
            objects: HashMap::new(),
            conjunction_hits: HashMap::new(),
            factor_last_access: HashMap::new(),
            current_tick: 0,
            lifecycles: HashMap::new(),
            completed_lifecycles: Vec::new(),
        }
    }

    // -----------------------------------------------------------------
    // Factor management (lifecycle tracking added)
    // -----------------------------------------------------------------

    pub fn add_factor(&mut self, factor: Factor) {
        for &obj_id in &factor.extent {
            self.boi.entry(obj_id).or_default().insert(factor.id);
        }
        for prop in &factor.intent {
            self.bpi.entry(prop.clone()).or_default().insert(factor.id);
        }
        self.factor_last_access.insert(factor.id, self.current_tick);

        // --- lifecycle recording ---
        self.lifecycles.insert(factor.id, FactorLifecycle {
            factor_id: factor.id,
            is_structural: factor.is_structural,
            created_at_tick: self.current_tick,
            last_accessed_tick: self.current_tick,
            evicted_at_tick: None,
        });

        self.factors.insert(factor.id, factor);
    }

    pub fn remove_factor(&mut self, factor_id: u64) {
        if let Some(factor) = self.factors.remove(&factor_id) {
            for obj_id in factor.extent {
                if let Some(set) = self.boi.get_mut(&obj_id) {
                    set.remove(&factor_id);
                }
            }
            for prop in factor.intent {
                if let Some(set) = self.bpi.get_mut(&prop) {
                    set.remove(&factor_id);
                }
            }
            self.factor_last_access.remove(&factor_id);

            // --- lifecycle: move to completed ---
            if let Some(mut lc) = self.lifecycles.remove(&factor_id) {
                lc.evicted_at_tick = Some(self.current_tick);
                self.completed_lifecycles.push(lc);
            }
        }
    }

    pub fn factors_with_property(&self, prop: &str) -> HashSet<u64> {
        self.bpi.get(prop).cloned().unwrap_or_default()
    }

    pub fn factors_for_object(&self, obj_id: ObjectId) -> HashSet<u64> {
        self.boi.get(&obj_id).cloned().unwrap_or_default()
    }

    // -----------------------------------------------------------------
    // Delta propagation (unchanged, but already calls record_factor_access)
    // -----------------------------------------------------------------

    pub fn apply_delta(&mut self, delta: &Delta, metrics: &Metrics) {
        match delta.delta_type {
            DeltaType::Insert => self.handle_insert(delta, metrics),
            DeltaType::Update => self.handle_update(delta, metrics),
            DeltaType::Delete => self.handle_delete(delta, metrics),
        }
    }

    fn handle_insert(&mut self, delta: &Delta, metrics: &Metrics) {
        let obj_id = delta.get_object_id();
        let props = delta.get_properties();
        self.objects.insert(obj_id, props.clone());

        let atoms: HashSet<String> = props.iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect();

        for atom in &atoms {
            if let Some(factor_ids) = self.bpi.get(atom) {
                let factor_ids: Vec<u64> = factor_ids.iter().cloned().collect();
                for fid in factor_ids {
                    if let Some(factor_mut) = self.factors.get_mut(&fid) {
                        if !factor_mut.extent.contains(&obj_id) && factor_is_satisfied_by(factor_mut, &props) {
                            factor_mut.extent.push(obj_id);
                            self.boi.entry(obj_id).or_default().insert(fid);
                            metrics.factor_ops.fetch_add(1, Ordering::Relaxed);
                            metrics.nodes_touched_by_updates.fetch_add(1, Ordering::Relaxed);
                            self.record_factor_access(fid);
                        }
                    }
                }
            }
        }
        metrics.objects_updated.fetch_add(1, Ordering::Relaxed);
    }

    fn handle_update(&mut self, delta: &Delta, metrics: &Metrics) {
        let obj_id = delta.get_object_id();
        let new_props = delta.get_properties();
        let old_props = self.objects.get(&obj_id).cloned().unwrap_or_default();

        let old_atoms: HashSet<String> = old_props.iter().map(|(k,v)| format!("{}={}", k, v)).collect();
        let new_atoms: HashSet<String> = new_props.iter().map(|(k,v)| format!("{}={}", k, v)).collect();
        let added: HashSet<_> = new_atoms.difference(&old_atoms).cloned().collect();
        let removed: HashSet<_> = old_atoms.difference(&new_atoms).cloned().collect();

        for atom in &removed {
            if let Some(factor_ids) = self.bpi.get(atom) {
                let factor_ids: Vec<u64> = factor_ids.iter().cloned().collect();
                for fid in factor_ids {
                    if let Some(factor) = self.factors.get_mut(&fid) {
                        if factor.extent.contains(&obj_id) {
                            if !factor_is_satisfied_by(factor, &new_props) {
                                factor.extent.retain(|id| *id != obj_id);
                                if let Some(boi_set) = self.boi.get_mut(&obj_id) {
                                    boi_set.remove(&fid);
                                }
                                metrics.factor_ops.fetch_add(1, Ordering::Relaxed);
                                metrics.nodes_touched_by_updates.fetch_add(1, Ordering::Relaxed);
                                self.record_factor_access(fid);
                            }
                        }
                    }
                }
            }
        }

        for atom in &added {
            if let Some(factor_ids) = self.bpi.get(atom) {
                let factor_ids: Vec<u64> = factor_ids.iter().cloned().collect();
                for fid in factor_ids {
                    if let Some(factor) = self.factors.get_mut(&fid) {
                        if !factor.extent.contains(&obj_id) && factor_is_satisfied_by(factor, &new_props) {
                            factor.extent.push(obj_id);
                            self.boi.entry(obj_id).or_default().insert(fid);
                            metrics.factor_ops.fetch_add(1, Ordering::Relaxed);
                            metrics.nodes_touched_by_updates.fetch_add(1, Ordering::Relaxed);
                            self.record_factor_access(fid);
                        }
                    }
                }
            }
        }

        self.objects.insert(obj_id, new_props);
        metrics.objects_updated.fetch_add(1, Ordering::Relaxed);
    }

    fn handle_delete(&mut self, delta: &Delta, metrics: &Metrics) {
        let obj_id = delta.get_object_id();
        if let Some(factor_ids) = self.boi.get(&obj_id) {
            let factor_ids: Vec<u64> = factor_ids.iter().cloned().collect();
            for fid in factor_ids {
                if let Some(factor) = self.factors.get_mut(&fid) {
                    factor.extent.retain(|id| *id != obj_id);
                    metrics.factor_ops.fetch_add(1, Ordering::Relaxed);
                    metrics.nodes_touched_by_updates.fetch_add(1, Ordering::Relaxed);
                    self.record_factor_access(fid);
                }
            }
        }
        self.boi.remove(&obj_id);
        self.objects.remove(&obj_id);
        metrics.objects_updated.fetch_add(1, Ordering::Relaxed);
    }

    // -----------------------------------------------------------------
    // Factor‑native query execution (with adaptation)
    // -----------------------------------------------------------------

    pub fn query(&mut self, filter: &QueryFilter, metrics: &Metrics) -> HashSet<ObjectId> {
        self.current_tick += 1;

        let result = self.eval_filter(filter);
        metrics.factor_ops.fetch_add(1, Ordering::Relaxed);
        metrics.total_queries.fetch_add(1, Ordering::Relaxed);

        self.record_factor_access_for_filter(filter);

        if let QueryFilter::And(sub_filters) = filter {
            self.adapt_conjunction(sub_filters, &result);
        }

        result
    }

    fn eval_filter(&self, filter: &QueryFilter) -> HashSet<ObjectId> {
        match filter {
            QueryFilter::Eq { attribute, value } => {
                let atom = format!("{}={}", attribute, value);
                let factor_ids = self.bpi.get(&atom).cloned().unwrap_or_default();
                let mut result = HashSet::new();
                for fid in factor_ids {
                    if let Some(factor) = self.factors.get(&fid) {
                        result.extend(&factor.extent);
                    }
                }
                result
            }
            QueryFilter::And(sub_filters) => {
                let mut iter = sub_filters.iter();
                if let Some(first) = iter.next() {
                    let mut result = self.eval_filter(first);
                    for f in iter {
                        result = result.intersection(&self.eval_filter(f)).cloned().collect();
                    }
                    result
                } else {
                    HashSet::new()
                }
            }
            QueryFilter::Or(sub_filters) => {
                let mut result = HashSet::new();
                for f in sub_filters {
                    result.extend(&self.eval_filter(f));
                }
                result
            }
        }
    }

    fn record_factor_access_for_filter(&mut self, filter: &QueryFilter) {
        match filter {
            QueryFilter::Eq { attribute, value } => {
                let atom = format!("{}={}", attribute, value);
                let fids: Vec<u64> = self.bpi
                    .get(&atom)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .collect();
                for fid in fids {
                    self.record_factor_access(fid);
                }
            }
            QueryFilter::And(sub) | QueryFilter::Or(sub) => {
                for f in sub {
                    self.record_factor_access_for_filter(f);
                }
            }
        }
    }

    fn record_factor_access(&mut self, factor_id: u64) {
        if let Some(factor) = self.factors.get_mut(&factor_id) {
            factor.access_count += 1;
        }
        self.factor_last_access.insert(factor_id, self.current_tick);
        // update lifecycle last access
        if let Some(lc) = self.lifecycles.get_mut(&factor_id) {
            lc.last_accessed_tick = self.current_tick;
        }
    }

    fn adapt_conjunction(&mut self, sub_filters: &[QueryFilter], current_extent: &HashSet<ObjectId>) {
        let mut atoms: Vec<String> = sub_filters.iter().filter_map(|f| {
            if let QueryFilter::Eq { attribute, value } = f {
                Some(format!("{}={}", attribute, value))
            } else {
                None
            }
        }).collect();
        if atoms.len() != sub_filters.len() {
            return;
        }
        atoms.sort();
        let key = atoms.join("&");

        let hits = self.conjunction_hits.entry(key.clone()).or_insert(0);
        *hits += 1;

        const THRESHOLD: u64 = 3;

        if *hits == THRESHOLD {
            let already_exists = self.factors.values().any(|f| {
                let mut sorted_intent = f.intent.clone();
                sorted_intent.sort();
                sorted_intent == atoms
            });
            if !already_exists {
                let new_id = self.factors.keys().max().unwrap_or(&0) + 1;
                let extent_vec: Vec<ObjectId> = current_extent.iter().cloned().collect();
                let factor = Factor {
                    id: new_id,
                    extent: extent_vec,
                    intent: atoms,
                    is_structural: false,
                    access_count: 0,
                    created_at: String::new(),
                    last_accessed: String::new(),
                };
                self.add_factor(factor);
                println!("[Adaptive] Materialised operational factor {} for {}", new_id, key);
            }
        }
    }

    pub fn evict_operational_factors(&mut self, ticks_threshold: u64) {
        let to_remove: Vec<u64> = self.factors.iter()
            .filter(|(id, f)| {
                f.is_structural == false
                    && self.factor_last_access.get(id).map_or(true, |last| self.current_tick - last > ticks_threshold)
            })
            .map(|(id, _)| *id)
            .collect();

        for id in to_remove {
            println!("[Eviction] Removing operational factor {}", id);
            self.remove_factor(id);
        }
    }

    // -----------------------------------------------------------------
    // Row reconstruction
    // -----------------------------------------------------------------

    pub fn reconstruct_rows(&self, object_ids: &HashSet<ObjectId>, metrics: &Metrics) -> Vec<HashMap<String, String>> {
        metrics.row_ops.fetch_add(1, Ordering::Relaxed);
        let mut rows = Vec::new();
        for &oid in object_ids {
            let mut row = HashMap::new();
            row.insert("id".to_string(), oid.to_string());
            if let Some(props) = self.objects.get(&oid) {
                for (attr, val) in props {
                    row.insert(attr.clone(), val.clone());
                }
            } else {
                let factor_ids = self.boi.get(&oid).cloned().unwrap_or_default();
                for fid in factor_ids {
                    if let Some(factor) = self.factors.get(&fid) {
                        for atom in &factor.intent {
                            if let Some((attr, val)) = atom.split_once('=') {
                                row.insert(attr.to_string(), val.to_string());
                            }
                        }
                    }
                }
            }
            rows.push(row);
        }
        rows
    }

    // -----------------------------------------------------------------
    // Metrics gathering (Phase 6)
    // -----------------------------------------------------------------

    pub fn gather_metrics(&self, metrics: &Metrics) -> MetricsReport {
        let mut active_lifecycles: Vec<FactorLifecycle> = self.lifecycles.values().cloned().collect();
        let mut evicted = self.completed_lifecycles.clone();

        // sort by factor id for readability
        active_lifecycles.sort_by_key(|lc| lc.factor_id);
        evicted.sort_by_key(|lc| lc.factor_id);

        let structural_count = self.factors.values().filter(|f| f.is_structural).count();
        let operational_count = self.factors.values().filter(|f| !f.is_structural).count();

        MetricsReport {
            total_queries: metrics.total_queries.load(Ordering::Relaxed),
            factor_ops: metrics.factor_ops.load(Ordering::Relaxed),
            row_ops: metrics.row_ops.load(Ordering::Relaxed),
            nodes_touched_by_updates: metrics.nodes_touched_by_updates.load(Ordering::Relaxed),
            objects_updated: metrics.objects_updated.load(Ordering::Relaxed),
            factor_utilization: metrics.factor_utilization(),
            uaf: metrics.uaf(),
            current_tick: self.current_tick,
            structural_factor_count: structural_count,
            operational_factor_count: operational_count,
            active_factors: active_lifecycles,
            evicted_factors: evicted,
        }
    }
}

// -----------------------------------------------------------------
// Helpers
// -----------------------------------------------------------------

fn factor_is_satisfied_by(factor: &Factor, object_props: &HashMap<String, String>) -> bool {
    for prop_atom in &factor.intent {
        if let Some((attr, val)) = prop_atom.split_once('=') {
            match object_props.get(attr) {
                Some(v) if v == val => continue,
                _ => return false,
            }
        } else {
            return false;
        }
    }
    true
}

impl Delta {
    pub fn get_object_id(&self) -> ObjectId {
        self.operation_details["id"].as_u64().unwrap_or(0) as u32
    }

    pub fn get_properties(&self) -> HashMap<String, String> {
        let mut props = HashMap::new();
        if let Some(obj) = self.operation_details.as_object() {
            for (k, v) in obj {
                if k != "id" {
                    if let Some(val_str) = v.as_str() {
                        props.insert(k.clone(), val_str.to_string());
                    }
                }
            }
        }
        props
    }
}
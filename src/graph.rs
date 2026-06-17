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
    pub materialization_threshold: u64,

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
            materialization_threshold: 3,
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

    /// Query with automatic row-reconstruction fallback for non-factorised attributes.
    /// Increments row_ops whenever any sub-filter had to leave factor space.
    pub fn query_with_fallback(&mut self, filter: &QueryFilter, metrics: &Metrics) -> HashSet<ObjectId> {
        self.current_tick += 1;

        let (result, used_rows) = self.eval_filter_with_fallback(filter);

        metrics.factor_ops.fetch_add(1, Ordering::Relaxed);
        metrics.total_queries.fetch_add(1, Ordering::Relaxed);
        if used_rows {
            metrics.row_ops.fetch_add(1, Ordering::Relaxed);
        }

        self.record_factor_access_for_filter(filter);

        // Only adapt when the conjunction is fully in factor space
        if !used_rows {
            if let QueryFilter::And(sub_filters) = filter {
                self.adapt_conjunction(sub_filters, &result);
            }
        }

        result
    }

    pub fn eval_filter_with_fallback(&self, filter: &QueryFilter) -> (HashSet<ObjectId>, bool) {
        match filter {
            QueryFilter::Eq { attribute, value } => {
                let atom = format!("{}={}", attribute, value);
                if let Some(factor_ids) = self.bpi.get(&atom) {
                    // Attribute is in factor space
                    let mut result = HashSet::new();
                    for fid in factor_ids {
                        if let Some(factor) = self.factors.get(fid) {
                            result.extend(&factor.extent);
                        }
                    }
                    (result, false)
                } else {
                    // Not in factor space — scan all objects (row reconstruction)
                    let mut result = HashSet::new();
                    for (id, props) in &self.objects {
                        if props.get(attribute.as_str()) == Some(value) {
                            result.insert(*id);
                        }
                    }
                    (result, true)
                }
            }
            QueryFilter::And(sub_filters) => {
                let mut sets: Vec<HashSet<ObjectId>> = Vec::new();
                let mut used_rows = false;
                for f in sub_filters {
                    let (res, used) = self.eval_filter_with_fallback(f);
                    sets.push(res);
                    used_rows |= used;
                }
                let mut iter = sets.into_iter();
                let first = iter.next().unwrap_or_default();
                let result = iter.fold(first, |acc, s| acc.intersection(&s).cloned().collect());
                (result, used_rows)
            }
            QueryFilter::Or(sub_filters) => {
                let mut result = HashSet::new();
                let mut used_rows = false;
                for f in sub_filters {
                    let (res, used) = self.eval_filter_with_fallback(f);
                    result.extend(res);
                    used_rows |= used;
                }
                (result, used_rows)
            }
        }
    }

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

        if *hits == self.materialization_threshold {
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

// -----------------------------------------------------------------
// Multi-table graph: coordinates per-table FactorGraphs for joins
// -----------------------------------------------------------------

pub struct MultiTableGraph {
    pub tables: HashMap<String, FactorGraph>,
    /// Global tick: incremented by apply_delta, query_table, and factor_join.
    pub current_tick: u64,
    /// Globally collected eviction records (aggregated from per-table evictions).
    pub completed_lifecycles: Vec<FactorLifecycle>,
}

impl MultiTableGraph {
    pub fn new() -> Self {
        MultiTableGraph {
            tables: HashMap::new(),
            current_tick: 0,
            completed_lifecycles: Vec::new(),
        }
    }

    pub fn add_table(&mut self, name: String, graph: FactorGraph) {
        self.tables.insert(name, graph);
    }

    pub fn table(&self, name: &str) -> &FactorGraph {
        self.tables.get(name).unwrap_or_else(|| panic!("table '{}' not found", name))
    }

    pub fn table_mut(&mut self, name: &str) -> &mut FactorGraph {
        self.tables.get_mut(name).unwrap_or_else(|| panic!("table '{}' not found", name))
    }

    /// The effective global tick: max of the explicit tick and all per-table ticks.
    pub fn max_tick(&self) -> u64 {
        let per_table = self.tables.values().map(|g| g.current_tick).max().unwrap_or(0);
        self.current_tick.max(per_table)
    }

    /// Advance the global tick and apply a delta to a specific table.
    pub fn apply_delta(&mut self, table: &str, delta: &Delta, metrics: &Metrics) {
        self.current_tick += 1;
        self.table_mut(table).apply_delta(delta, metrics);
    }

    /// Run a single-table query via the factor-native path and advance the global tick.
    pub fn query_table(&mut self, table: &str, filter: &QueryFilter, metrics: &Metrics) -> HashSet<ObjectId> {
        self.current_tick += 1;
        self.table_mut(table).query_with_fallback(filter, metrics)
    }

    /// Evict stale operational factors from every table, collecting their lifecycle records.
    pub fn evict_all(&mut self, ticks_threshold: u64) {
        for graph in self.tables.values_mut() {
            graph.evict_operational_factors(ticks_threshold);
            // Harvest newly completed lifecycles into the global list.
            self.completed_lifecycles.extend(graph.completed_lifecycles.drain(..));
        }
    }

    /// Evaluate a slice of filters against a single table, returning the matching
    /// object IDs and whether any row-level fallback was required.
    pub fn eval_table_filter(
        &self,
        table: &str,
        filters: &[QueryFilter],
        metrics: &Metrics,
    ) -> (HashSet<ObjectId>, bool) {
        let tg = self.table(table);
        if filters.is_empty() {
            return (tg.objects.keys().cloned().collect(), false);
        }
        let mut result: HashSet<ObjectId> = tg.objects.keys().cloned().collect();
        let mut used_rows = false;

        for filter in filters {
            match filter {
                QueryFilter::Eq { attribute, value } => {
                    let atom = format!("{}={}", attribute, value);
                    if let Some(fids) = tg.bpi.get(&atom) {
                        let mut matches: HashSet<ObjectId> = HashSet::new();
                        for fid in fids {
                            if let Some(factor) = tg.factors.get(fid) {
                                matches.extend(&factor.extent);
                            }
                        }
                        result = result.intersection(&matches).cloned().collect();
                    } else {
                        result = result.into_iter().filter(|id| {
                            tg.objects.get(id)
                                .and_then(|props| props.get(attribute))
                                .map(|v| v == value)
                                .unwrap_or(false)
                        }).collect();
                        used_rows = true;
                    }
                }
                QueryFilter::And(sub) => {
                    for sub_f in sub {
                        let (sub_res, sub_used) = self.eval_table_filter(table, &[sub_f.clone()], metrics);
                        result = result.intersection(&sub_res).cloned().collect();
                        used_rows |= sub_used;
                    }
                }
                QueryFilter::Or(sub) => {
                    let mut union_res: HashSet<ObjectId> = HashSet::new();
                    for sub_f in sub {
                        let (sub_res, sub_used) = self.eval_table_filter(table, &[sub_f.clone()], metrics);
                        union_res.extend(sub_res);
                        used_rows |= sub_used;
                    }
                    result = result.intersection(&union_res).cloned().collect();
                }
            }
        }
        (result, used_rows)
    }

    /// Factor-level equi-join between two tables on a shared attribute value.
    /// When both sides have factors covering the join attribute the join stays
    /// entirely in factor space (factor_ops++).  If either side has no such
    /// factors we fall back to a nested-loop row join (row_ops++).
    /// Returns the set of (left_id, right_id) pairs and whether rows were used.
    pub fn factor_join(
        &self,
        left_table: &str,
        right_table: &str,
        join_attr: &str,
        left_filters: &[QueryFilter],
        right_filters: &[QueryFilter],
        metrics: &Metrics,
    ) -> (HashSet<(ObjectId, ObjectId)>, bool) {
        let (left_universe, left_row)   = self.eval_table_filter(left_table,  left_filters,  metrics);
        let (right_universe, right_row) = self.eval_table_filter(right_table, right_filters, metrics);
        let used_rows = left_row || right_row;

        let left_graph  = self.table(left_table);
        let right_graph = self.table(right_table);
        let join_prefix = format!("{}=", join_attr);

        // Collect factors from each side that carry the join attribute in their intent.
        let left_factors: Vec<&Factor> = left_graph.factors.values()
            .filter(|f| f.intent.iter().any(|a| a.starts_with(&join_prefix)))
            .collect();
        let right_factors: Vec<&Factor> = right_graph.factors.values()
            .filter(|f| f.intent.iter().any(|a| a.starts_with(&join_prefix)))
            .collect();

        if left_factors.is_empty() || right_factors.is_empty() {
            // Row-based fallback: nested-loop join on the raw object stores.
            metrics.row_ops.fetch_add(1, Ordering::Relaxed);
            metrics.total_queries.fetch_add(1, Ordering::Relaxed);
            let mut result = HashSet::new();
            for &l in &left_universe {
                let lval = left_graph.objects.get(&l).and_then(|p| p.get(join_attr));
                if lval.is_none() { continue; }
                for &r in &right_universe {
                    if right_graph.objects.get(&r).and_then(|p| p.get(join_attr)) == lval {
                        result.insert((l, r));
                    }
                }
            }
            return (result, true);
        }

        // Factor-level join: match factors by the value of the join attribute.
        let mut result = HashSet::new();
        for lf in &left_factors {
            let lval = lf.intent.iter()
                .find(|a| a.starts_with(&join_prefix))
                .and_then(|a| a.splitn(2, '=').nth(1))
                .unwrap_or("");

            let l_extent: HashSet<ObjectId> = lf.extent.iter()
                .cloned()
                .filter(|id| left_universe.contains(id))
                .collect();
            if l_extent.is_empty() { continue; }

            for rf in &right_factors {
                let rval = rf.intent.iter()
                    .find(|a| a.starts_with(&join_prefix))
                    .and_then(|a| a.splitn(2, '=').nth(1))
                    .unwrap_or("");
                if lval != rval { continue; }

                let r_extent: HashSet<ObjectId> = rf.extent.iter()
                    .cloned()
                    .filter(|id| right_universe.contains(id))
                    .collect();
                if r_extent.is_empty() { continue; }

                for &l in &l_extent {
                    for &r in &r_extent {
                        result.insert((l, r));
                    }
                }
            }
        }

        metrics.factor_ops.fetch_add(1, Ordering::Relaxed);
        metrics.total_queries.fetch_add(1, Ordering::Relaxed);
        (result, used_rows)
    }

    pub fn gather_metrics(&self, metrics: &Metrics) -> MetricsReport {
        let mut structural  = 0usize;
        let mut operational = 0usize;
        let mut active_lcs: Vec<FactorLifecycle> = Vec::new();
        // Evicted = global list + anything still in per-table lists.
        let mut evicted_lcs: Vec<FactorLifecycle> = self.completed_lifecycles.clone();

        for graph in self.tables.values() {
            structural  += graph.factors.values().filter(|f|  f.is_structural).count();
            operational += graph.factors.values().filter(|f| !f.is_structural).count();
            active_lcs.extend(graph.lifecycles.values().cloned());
            evicted_lcs.extend(graph.completed_lifecycles.iter().cloned());
        }

        active_lcs.sort_by_key(|lc| lc.factor_id);
        evicted_lcs.sort_by_key(|lc| lc.factor_id);
        evicted_lcs.dedup_by_key(|lc| lc.factor_id);

        MetricsReport {
            total_queries: metrics.total_queries.load(Ordering::Relaxed),
            factor_ops:    metrics.factor_ops.load(Ordering::Relaxed),
            row_ops:       metrics.row_ops.load(Ordering::Relaxed),
            nodes_touched_by_updates: metrics.nodes_touched_by_updates.load(Ordering::Relaxed),
            objects_updated: metrics.objects_updated.load(Ordering::Relaxed),
            factor_utilization: metrics.factor_utilization(),
            uaf:  metrics.uaf(),
            current_tick: self.max_tick(),
            structural_factor_count:  structural,
            operational_factor_count: operational,
            active_factors:  active_lcs,
            evicted_factors: evicted_lcs,
        }
    }
}

// -----------------------------------------------------------------

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
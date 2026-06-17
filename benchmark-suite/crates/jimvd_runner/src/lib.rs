use anyhow::Result;
use data_generator::{User, clearance_name, dept_name, region_name, role_name};
use jimvd::cover::GreedyCover;
use jimvd::graph::{FactorGraph, MultiTableGraph};
use jimvd::metrics::Metrics as JimMetrics;
use jimvd::types::{Delta, DeltaType, QueryFilter};
use runner_api::{DatabaseRunner, OpResult};
use std::sync::atomic::Ordering;
use workload_generator::{FilterPredicate, Operation};

pub struct JimvdRunner {
    graph:         MultiTableGraph,
    jmetrics:      JimMetrics,
    uaf_tracker:   metrics::UafTracker,
    latencies:     metrics::LatencyHistogram,
    dataset_size:  usize,
    next_delta_id: i64,
    table_ready:   bool,
}

impl JimvdRunner {
    pub fn new(_materialization_threshold: usize, _eviction_ticks: u64) -> Self {
        JimvdRunner {
            graph:         MultiTableGraph::new(),
            jmetrics:      JimMetrics::new(),
            uaf_tracker:   metrics::UafTracker::new(),
            latencies:     metrics::LatencyHistogram::new(),
            dataset_size:  0,
            next_delta_id: 1,
            table_ready:   false,
        }
    }

    fn ensure_table(&mut self) {
        if !self.table_ready {
            self.graph.add_table("users".to_string(), FactorGraph::new());
            self.table_ready = true;
        }
    }

    fn ensure_named_table(&mut self, name: &str) {
        if !self.graph.tables.contains_key(name) {
            self.graph.add_table(name.to_string(), FactorGraph::new());
        }
    }

    fn make_delta(&mut self, dt: DeltaType, details: serde_json::Value) -> Delta {
        let id = self.next_delta_id;
        self.next_delta_id += 1;
        Delta {
            delta_id:          id,
            db_id:             1,
            base_version:      "v1".to_string(),
            sequence:          id as i32,
            delta_type:        dt,
            table_name:        "users".to_string(),
            codomain_ids:      vec![],
            contact_ids:       vec![],
            operation_details: details,
        }
    }

    fn convert_filter(pred: &FilterPredicate) -> QueryFilter {
        match pred {
            FilterPredicate::Eq { attribute, value } => QueryFilter::Eq {
                attribute: attribute.clone(),
                value:     value.clone(),
            },
            FilterPredicate::And(subs) => QueryFilter::And(
                subs.iter().map(Self::convert_filter).collect(),
            ),
            FilterPredicate::Or(subs) => QueryFilter::Or(
                subs.iter().map(Self::convert_filter).collect(),
            ),
        }
    }
}

impl DatabaseRunner for JimvdRunner {
    fn name(&self) -> &str { "jimvd" }

    fn load_data(&mut self, users: &[User]) -> Result<()> {
        self.ensure_table();
        self.dataset_size = users.len();
        for user in users {
            let details = serde_json::json!({
                "id":         user.id as u32,
                "role":       role_name(user.role),
                "region":     region_name(user.region),
                "department": dept_name(user.department),
                "clearance":  clearance_name(user.clearance),
                "tenant":     user.tenant.to_string(),
            });
            let delta = self.make_delta(DeltaType::Insert, details);
            self.graph.apply_delta("users", &delta, &self.jmetrics);
        }
        // Build structural factors over IAM attributes only (role/region/department).
        // Clearance and tenant are intentionally left un-factorised so that Compliance,
        // Tenant, and Security phases exercise the cold-start row-scan path.
        const IAM_ATTRS: &[&str] = &["role", "region", "department"];
        let seed: Vec<(u32, std::collections::HashMap<String, String>)> = self
            .graph
            .tables
            .get("users")
            .map(|g| {
                g.objects
                    .iter()
                    .map(|(&oid, props)| {
                        let iam: std::collections::HashMap<String, String> = props
                            .iter()
                            .filter(|(k, _)| IAM_ATTRS.contains(&k.as_str()))
                            .map(|(k, v)| (k.clone(), v.clone()))
                            .collect();
                        (oid, iam)
                    })
                    .collect()
            })
            .unwrap_or_default();
        let n = seed.len();
        let mut cover = GreedyCover::new(seed);
        let factors = cover.build_factors();
        let f = factors.len();
        if let Some(g) = self.graph.tables.get_mut("users") {
            for factor in factors {
                g.add_factor(factor);
            }
        }
        log::info!("jimvd_runner: loaded {} users → {} structural factors", n, f);
        Ok(())
    }

    fn execute(&mut self, op: &Operation) -> Result<OpResult> {
        self.ensure_table();
        let before_nodes = self.jmetrics.nodes_touched_by_updates.load(Ordering::Relaxed);
        let start = std::time::Instant::now();

        match op {
            Operation::PointLookup { id } => {
                let filter = QueryFilter::Eq {
                    attribute: "id".to_string(),
                    value:     id.to_string(),
                };
                let _ = self.graph.query_table("users", &filter, &self.jmetrics);
            }
            Operation::EqualityFilter { attribute, value } => {
                let filter = QueryFilter::Eq {
                    attribute: attribute.clone(),
                    value:     value.clone(),
                };
                let _ = self.graph.query_table("users", &filter, &self.jmetrics);
            }
            Operation::MultiAttributeFilter { predicates } => {
                let sub_filters: Vec<QueryFilter> =
                    predicates.iter().map(Self::convert_filter).collect();
                let filter = QueryFilter::And(sub_filters);
                let _ = self.graph.query_table("users", &filter, &self.jmetrics);
            }
            Operation::Insert { user } => {
                let details = serde_json::json!({
                    "id":         user.id as u32,
                    "role":       role_name(user.role),
                    "region":     region_name(user.region),
                    "department": dept_name(user.department),
                    "clearance":  clearance_name(user.clearance),
                    "tenant":     user.tenant.to_string(),
                });
                let delta = self.make_delta(DeltaType::Insert, details);
                self.graph.apply_delta("users", &delta, &self.jmetrics);
            }
            Operation::Update { user_id, attribute, new_value } => {
                let existing: std::collections::HashMap<String, String> = self
                    .graph
                    .tables
                    .get("users")
                    .and_then(|g| g.objects.get(&(*user_id as u32)))
                    .cloned()
                    .unwrap_or_default();
                let mut details = serde_json::Map::new();
                details.insert("id".to_string(), serde_json::Value::Number((*user_id as u32).into()));
                for (k, v) in existing {
                    details.insert(k, serde_json::Value::String(v));
                }
                details.insert(attribute.clone(), serde_json::Value::String(new_value.clone()));
                let delta = self.make_delta(DeltaType::Update, serde_json::Value::Object(details));
                self.graph.apply_delta("users", &delta, &self.jmetrics);
            }
            Operation::Delete { user_id } => {
                let details = serde_json::json!({ "id": *user_id as u32 });
                let delta = self.make_delta(DeltaType::Delete, details);
                self.graph.apply_delta("users", &delta, &self.jmetrics);
            }
            Operation::Join { left_table, right_table, join_attribute } => {
                // Ensure both tables exist before joining
                self.ensure_named_table(left_table.as_str());
                self.ensure_named_table(right_table.as_str());
                let _ = self.graph.factor_join(
                    left_table,
                    right_table,
                    join_attribute,
                    &[],
                    &[],
                    &self.jmetrics,
                );
            }
        }

        let latency = start.elapsed();
        self.latencies.record(latency);

        let after_nodes = self.jmetrics.nodes_touched_by_updates.load(Ordering::Relaxed);
        let nodes_touched = after_nodes.saturating_sub(before_nodes) as usize;
        if nodes_touched > 0 {
            self.uaf_tracker.record_update(nodes_touched);
        }

        Ok(OpResult { latency, nodes_touched })
    }

    fn collect_metrics(&self) -> metrics::Metrics {
        let report = self.graph.gather_metrics(&self.jmetrics);
        // Clone histogram so we can call percentile() (which sorts in place)
        let mut hist = self.latencies.clone();
        metrics::Metrics {
            p50_latency_us:     hist.percentile(50.0),
            p95_latency_us:     hist.percentile(95.0),
            p99_latency_us:     hist.percentile(99.0),
            throughput_ops_sec: 0.0,
            storage_bytes:      0,
            metadata_bytes:     0,
            factor_utilization: report.factor_utilization,
            uaf:                self.uaf_tracker.uaf(),
            factor_count:       report.structural_factor_count + report.operational_factor_count,
            graph_nodes:        report.structural_factor_count + report.operational_factor_count,
            memory_peak_bytes:  0,
        }
    }

    fn reset_metrics(&mut self) {
        self.jmetrics.reset();
        self.uaf_tracker.reset();
        self.latencies = metrics::LatencyHistogram::new();
    }
}

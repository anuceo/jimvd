use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Deserialize)]
pub struct WorkloadConfig {
    pub workload_name: String,
    pub description: String,
    pub tables: HashMap<String, TableSpec>,
    pub query_mix: Vec<QueryTemplate>,
    pub write_mix: WriteMix,
    pub run_options: RunOptions,
    /// Master RNG seed. Every RNG in the system is derived from this value so
    /// that runs are reproducible. Defaults to 0 to preserve historical runs.
    #[serde(default)]
    pub rng_seed: u64,
}

#[derive(Debug, Deserialize)]
pub struct TableSpec {
    pub attributes: HashMap<String, Vec<String>>,
    pub initial_objects: usize,
    #[serde(default = "default_correlation")]
    pub correlation_hint: String,
    /// Optional per-attribute sampling weights, parallel to the value list in
    /// `attributes`. When present for an attribute, values are sampled using
    /// these weights instead of uniformly. Missing entries stay uniform.
    #[serde(default)]
    pub weights: HashMap<String, Vec<f64>>,
    /// Optional per-attribute probability (0.0–1.0) of emitting NULL instead of
    /// a sampled value. Missing entries never emit NULL.
    #[serde(default)]
    pub null_probability: HashMap<String, f64>,
    /// Optional continuous (integer range) attributes. These are generated as a
    /// random integer in [min, max]; they are intentionally high-cardinality and
    /// not factorised, so queries against them fall back to row scans.
    #[serde(default)]
    pub continuous: HashMap<String, ContinuousSpec>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ContinuousSpec {
    pub min: i64,
    pub max: i64,
}

fn default_correlation() -> String {
    "low".to_string()
}

/// A weighted "hot" value for a query template, e.g. a value that should appear
/// far more often than the others.
#[derive(Debug, Deserialize, Clone)]
pub struct HotValue {
    pub value: String,
    pub probability: f64,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum QueryTemplate {
    #[serde(rename = "eq")]
    Eq {
        weight: u32,
        attribute: String,
        values: Vec<String>,
        /// Optional hot-spot distribution. When present, the listed values are
        /// chosen with their associated probabilities; any remaining
        /// probability mass falls back to a uniform pick over `values`.
        #[serde(default)]
        hot_values: Vec<HotValue>,
    },
    #[serde(rename = "and")]
    And {
        weight: u32,
        attributes: Vec<String>,
    },
    #[serde(rename = "or")]
    Or {
        weight: u32,
        attributes: Vec<String>,
        values: Vec<String>,
    },
}

#[derive(Debug, Deserialize)]
pub struct WriteMix {
    pub insert_rate: f64,
    pub update_rate: f64,
    pub delete_rate: f64,
    pub attributes: Vec<String>,
    /// Optional per-attribute mutation weights. When updating, the attribute to
    /// mutate is chosen proportionally to these weights instead of uniformly.
    /// Attributes absent from the map default to weight 0 unless the map itself
    /// is empty (in which case selection stays uniform over `attributes`).
    #[serde(default)]
    pub attribute_weights: HashMap<String, f64>,
}

#[derive(Debug, Deserialize)]
pub struct RunOptions {
    pub total_operations: usize,
    pub metrics_interval_ops: usize,
    pub warmup_ops: usize,
    pub adaptation_enabled: bool,
}

impl QueryTemplate {
    pub fn weight(&self) -> u32 {
        match self {
            QueryTemplate::Eq { weight, .. } => *weight,
            QueryTemplate::And { weight, .. } => *weight,
            QueryTemplate::Or { weight, .. } => *weight,
        }
    }
}

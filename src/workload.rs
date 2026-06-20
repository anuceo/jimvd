use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum AttributeDef {
    Simple(Vec<String>),
    Extended(AttributeSpec),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub enum AttributeSpec {
    #[serde(rename = "categorical")]
    Categorical {
        values: Vec<String>,
        #[serde(default)]
        weights: Option<Vec<f64>>,
        #[serde(default)]
        null_probability: f64,
    },
    #[serde(rename = "continuous")]
    Continuous {
        min: i64,
        max: i64,
        #[serde(default)]
        null_probability: f64,
    },
}

#[derive(Debug, Clone, Deserialize)]
pub struct HotValue {
    pub value: String,
    pub weight: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AdaptationConfig {
    #[serde(default = "default_materialization_threshold")]
    pub materialization_threshold: u64,
    #[serde(default = "default_eviction_ticks")]
    pub eviction_ticks: u64,
}

fn default_materialization_threshold() -> u64 { 3 }
fn default_eviction_ticks() -> u64 { 500 }

impl Default for AdaptationConfig {
    fn default() -> Self {
        AdaptationConfig {
            materialization_threshold: default_materialization_threshold(),
            eviction_ticks: default_eviction_ticks(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorkloadConfig {
    pub workload_name: String,
    pub description: String,
    pub tables: HashMap<String, TableSpec>,
    pub query_mix: Vec<QueryTemplate>,
    pub write_mix: WriteMix,
    pub run_options: RunOptions,
    #[serde(default)]
    pub adaptation: AdaptationConfig,
    #[serde(default)]
    pub rng_seed: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TableSpec {
    pub attributes: HashMap<String, AttributeDef>,
    pub initial_objects: usize,
    #[serde(default = "default_correlation")]
    pub correlation_hint: String,
    #[serde(default)]
    pub factorize_attributes: Option<Vec<String>>,
}

fn default_correlation() -> String {
    "low".to_string()
}

fn default_query_table() -> String {
    "employees".to_string()
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub enum QueryTemplate {
    #[serde(rename = "eq")]
    Eq {
        #[serde(default)]
        weight: u32,
        attribute: String,
        values: Vec<String>,
        #[serde(default = "default_query_table")]
        table: String,
        #[serde(default)]
        hot_values: Vec<HotValue>,
    },
    #[serde(rename = "and")]
    And {
        #[serde(default)]
        weight: u32,
        attributes: Vec<String>,
        #[serde(default = "default_query_table")]
        table: String,
    },
    #[serde(rename = "or")]
    Or {
        #[serde(default)]
        weight: u32,
        attributes: Vec<String>,
        values: Vec<String>,
        #[serde(default = "default_query_table")]
        table: String,
        #[serde(default)]
        hot_values: Vec<HotValue>,
    },
    #[serde(rename = "join")]
    Join {
        weight: u32,
        left_table: String,
        right_table: String,
        join_attribute: String,
        #[serde(default)]
        left_filters: Vec<QueryTemplate>,
        #[serde(default)]
        right_filters: Vec<QueryTemplate>,
    },
}

#[derive(Debug, Clone, Deserialize)]
pub struct WriteMix {
    pub insert_rate: f64,
    pub update_rate: f64,
    pub delete_rate: f64,
    pub attributes: Vec<String>,
    /// Which table writes go to (defaults to "employees" for backward compat).
    #[serde(default = "default_query_table")]
    pub table: String,
    #[serde(default)]
    pub attribute_weights: Option<HashMap<String, f64>>,
}

#[derive(Debug, Clone, Deserialize)]
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
            QueryTemplate::Join { weight, .. } => *weight,
        }
    }
}

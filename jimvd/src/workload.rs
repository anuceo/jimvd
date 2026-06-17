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
}

#[derive(Debug, Deserialize)]
pub struct TableSpec {
    pub attributes: HashMap<String, Vec<String>>,
    pub initial_objects: usize,
    #[serde(default = "default_correlation")]
    pub correlation_hint: String,
}

fn default_correlation() -> String {
    "low".to_string()
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum QueryTemplate {
    #[serde(rename = "eq")]
    Eq {
        weight: u32,
        attribute: String,
        values: Vec<String>,
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
}

#[derive(Debug, Deserialize)]
pub struct RunOptions {
    pub total_operations: usize,
    pub metrics_interval_ops: usize,
    pub warmup_ops: usize,
    pub adaptation_enabled: bool,
}
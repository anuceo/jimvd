use serde::{Deserialize, Serialize};
use uuid::Uuid;
use std::collections::HashMap;

/// A property atom, e.g. "Role=Admin"
pub type PropertyAtom = String;

/// Object (row) identifier
pub type ObjectId = u32;

/// A logical factor: extent (set of object IDs) and intent (set of property atoms)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Factor {
    pub id: u64,
    pub extent: Vec<ObjectId>,   // will be replaced with bitmap later
    pub intent: Vec<PropertyAtom>,
    pub is_structural: bool,     // true = from factorization; false = operational/materialized
    pub access_count: u64,
    pub created_at: String,
    pub last_accessed: String,
}

/// Types of nodes in the dependency graph
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NodeType {
    Object(ObjectId),
    Factor(u64),
    JoinFactor(u64),
    Codomain(i32),
    Contact(i32),
    PropertyAtom(String),
}

/// A node in the dependency graph
#[derive(Debug, Clone)]
pub struct GraphNode {
    pub node_type: NodeType,
    pub edges_out: Vec<NodeType>,   // nodes this depends on (outgoing)
    pub edges_in: Vec<NodeType>,    // nodes that depend on this (incoming)
}

/// The dependency graph
#[derive(Debug, Clone)]
pub struct DependencyGraph {
    pub nodes: HashMap<NodeType, GraphNode>,
}

impl DependencyGraph {
    pub fn new() -> Self {
        DependencyGraph {
            nodes: HashMap::new(),
        }
    }
}

/// Simple filter for factor‑space queries
#[derive(Debug, Clone)]
pub enum QueryFilter {
    /// Attribute equals value, e.g. Role = "Admin"
    Eq { attribute: String, value: String },
    /// All sub‑filters must match (AND)
    And(Vec<QueryFilter>),
    /// Any sub‑filter must match (OR)
    Or(Vec<QueryFilter>),
}

/// Codomain definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Codomain {
    pub id: i32,
    pub name: String,
    pub filter_condition: serde_json::Value,  // e.g. {"Role":"Admin"}
    pub tables: Vec<String>,
}

/// Contact relation (named set membership)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContactRelation {
    pub id: i32,
    pub department: String,
    pub doctor_name: String,
    pub object_ids: Vec<ObjectId>,
    pub location_ids: Vec<i32>,
}

/// Delta record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Delta {
    pub delta_id: i64,
    pub db_id: i32,
    pub base_version: String,
    pub sequence: i32,
    pub delta_type: DeltaType,
    pub table_name: String,
    pub codomain_ids: Vec<i32>,
    pub contact_ids: Vec<i32>,
    pub operation_details: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DeltaType {
    Insert,
    Update,
    Delete,
}

/// A user session
#[derive(Debug, Clone)]
pub struct Session {
    pub session_id: Uuid,
    pub db_id: i32,
    pub user_id: i32,
    pub visible_base_version: String,
    pub visible_deltas: Vec<i64>,
}

/// Lifecycle record for a factor (used to compute half‑life etc.)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FactorLifecycle {
    pub factor_id: u64,
    pub is_structural: bool,
    pub created_at_tick: u64,
    pub last_accessed_tick: u64,
    pub evicted_at_tick: Option<u64>,
}

/// Aggregated metrics snapshot – serialisable for benchmarking.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsReport {
    pub total_queries: u64,
    /// Factor‑space ops on the query path that stayed in factor space.
    pub query_factor_ops: u64,
    /// Factor‑space ops on the write path (incremental maintenance).
    pub write_factor_ops: u64,
    pub row_ops: u64,
    /// Graph nodes touched during write/delta propagation (was
    /// `nodes_touched_by_updates`).
    pub write_propagation_nodes: u64,
    pub objects_updated: u64,
    /// Combined utilization across query and write factor ops (backward
    /// compatible definition).
    pub factor_utilization: f64,
    /// Query‑only utilization: query_factor_ops / (query_factor_ops + row_ops).
    pub query_factor_utilization: f64,
    pub uaf: f64,
    pub current_tick: u64,
    pub structural_factor_count: usize,
    pub operational_factor_count: usize,
    /// Resident set size (bytes) of the process at snapshot time.
    pub memory_bytes: u64,
    /// Estimated logical storage of all factors plus BOI/BPI overhead (bytes).
    pub storage_bytes: u64,
    pub active_factors: Vec<FactorLifecycle>,
    pub evicted_factors: Vec<FactorLifecycle>,
}
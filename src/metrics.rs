use std::sync::atomic::{AtomicU64, Ordering};

/// Global metrics for factor‑native execution and incremental maintenance.
pub struct Metrics {
    /// Total number of queries executed.
    pub total_queries: AtomicU64,
    /// Number of factor‑space operations (extent union/intersect, factor selection, etc.).
    pub factor_ops: AtomicU64,
    /// Number of row‑level operations (reconstruction, fallback scans).
    pub row_ops: AtomicU64,
    /// Graph nodes touched during delta propagation.
    pub nodes_touched_by_updates: AtomicU64,
    /// Number of objects that were inserted, updated, or deleted.
    pub objects_updated: AtomicU64,
}

impl Metrics {
    pub fn new() -> Self {
        Metrics {
            total_queries: AtomicU64::new(0),
            factor_ops: AtomicU64::new(0),
            row_ops: AtomicU64::new(0),
            nodes_touched_by_updates: AtomicU64::new(0),
            objects_updated: AtomicU64::new(0),
        }
    }

    /// Factor Utilization = factor_ops / (factor_ops + row_ops)
    pub fn factor_utilization(&self) -> f64 {
        let f = self.factor_ops.load(Ordering::Relaxed) as f64;
        let r = self.row_ops.load(Ordering::Relaxed) as f64;
        if f + r == 0.0 {
            0.0
        } else {
            f / (f + r)
        }
    }

    /// Update Amplification Factor = nodes_touched / objects_updated
    pub fn uaf(&self) -> f64 {
        let n = self.nodes_touched_by_updates.load(Ordering::Relaxed) as f64;
        let o = self.objects_updated.load(Ordering::Relaxed) as f64;
        if o == 0.0 {
            0.0
        } else {
            n / o
        }
    }
}
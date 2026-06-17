use std::sync::atomic::{AtomicU64, Ordering};

/// Global metrics for factor‑native execution and incremental maintenance.
pub struct Metrics {
    /// Total number of queries executed.
    pub total_queries: AtomicU64,
    /// Factor‑space operations performed on the query path (extent
    /// union/intersect, factor selection) that stayed in factor space.
    pub query_factor_ops: AtomicU64,
    /// Factor‑space operations performed on the write path (incremental
    /// maintenance during insert/update/delete propagation).
    pub write_factor_ops: AtomicU64,
    /// Number of row‑level operations (reconstruction, fallback scans).
    pub row_ops: AtomicU64,
    /// Graph nodes touched during delta propagation.
    pub write_propagation_nodes: AtomicU64,
    /// Number of objects that were inserted, updated, or deleted.
    pub objects_updated: AtomicU64,
}

impl Metrics {
    pub fn new() -> Self {
        Metrics {
            total_queries: AtomicU64::new(0),
            query_factor_ops: AtomicU64::new(0),
            write_factor_ops: AtomicU64::new(0),
            row_ops: AtomicU64::new(0),
            write_propagation_nodes: AtomicU64::new(0),
            objects_updated: AtomicU64::new(0),
        }
    }

    /// Combined Factor Utilization (backward compatible):
    /// (query_factor_ops + write_factor_ops) /
    /// (query_factor_ops + write_factor_ops + row_ops)
    pub fn factor_utilization(&self) -> f64 {
        let q = self.query_factor_ops.load(Ordering::Relaxed) as f64;
        let w = self.write_factor_ops.load(Ordering::Relaxed) as f64;
        let r = self.row_ops.load(Ordering::Relaxed) as f64;
        let f = q + w;
        if f + r == 0.0 { 0.0 } else { f / (f + r) }
    }

    /// Query‑only Factor Utilization:
    /// query_factor_ops / (query_factor_ops + row_ops)
    pub fn query_factor_utilization(&self) -> f64 {
        let q = self.query_factor_ops.load(Ordering::Relaxed) as f64;
        let r = self.row_ops.load(Ordering::Relaxed) as f64;
        if q + r == 0.0 { 0.0 } else { q / (q + r) }
    }

    /// Update Amplification Factor = write_propagation_nodes / objects_updated
    pub fn uaf(&self) -> f64 {
        let n = self.write_propagation_nodes.load(Ordering::Relaxed) as f64;
        let o = self.objects_updated.load(Ordering::Relaxed) as f64;
        if o == 0.0 { 0.0 } else { n / o }
    }
}

/// Current resident set size (RSS) of this process in bytes, via `sysinfo`.
/// Returns 0 if the process cannot be inspected.
pub fn get_current_memory_usage() -> u64 {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};

    let Ok(pid) = sysinfo::get_current_pid() else {
        return 0;
    };
    let mut sys = System::new();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        true,
        ProcessRefreshKind::nothing().with_memory(),
    );
    sys.process(pid).map(|p| p.memory()).unwrap_or(0)
}

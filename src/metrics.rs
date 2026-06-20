use std::sync::atomic::{AtomicU64, Ordering};

pub struct Metrics {
    pub total_queries:           AtomicU64,
    pub query_factor_ops:        AtomicU64,
    pub write_factor_ops:        AtomicU64,
    pub row_ops:                 AtomicU64,
    pub write_propagation_nodes: AtomicU64,
    pub objects_updated:         AtomicU64,
    pub join_fallbacks:          AtomicU64,
}

impl Metrics {
    pub fn new() -> Self {
        Metrics {
            total_queries:           AtomicU64::new(0),
            query_factor_ops:        AtomicU64::new(0),
            write_factor_ops:        AtomicU64::new(0),
            row_ops:                 AtomicU64::new(0),
            write_propagation_nodes: AtomicU64::new(0),
            objects_updated:         AtomicU64::new(0),
            join_fallbacks:          AtomicU64::new(0),
        }
    }

    pub fn factor_utilization(&self) -> f64 {
        let q = self.query_factor_ops.load(Ordering::Relaxed) as f64;
        let w = self.write_factor_ops.load(Ordering::Relaxed) as f64;
        let r = self.row_ops.load(Ordering::Relaxed) as f64;
        let denom = q + w + r;
        if denom == 0.0 { 0.0 } else { (q + w) / denom }
    }

    pub fn query_factor_utilization(&self) -> f64 {
        let q = self.query_factor_ops.load(Ordering::Relaxed) as f64;
        let r = self.row_ops.load(Ordering::Relaxed) as f64;
        if q + r == 0.0 { 0.0 } else { q / (q + r) }
    }

    pub fn reset(&self) {
        self.total_queries.store(0, Ordering::Relaxed);
        self.query_factor_ops.store(0, Ordering::Relaxed);
        self.write_factor_ops.store(0, Ordering::Relaxed);
        self.row_ops.store(0, Ordering::Relaxed);
        self.write_propagation_nodes.store(0, Ordering::Relaxed);
        self.objects_updated.store(0, Ordering::Relaxed);
        self.join_fallbacks.store(0, Ordering::Relaxed);
    }

    pub fn uaf(&self) -> f64 {
        let n = self.write_propagation_nodes.load(Ordering::Relaxed) as f64;
        let o = self.objects_updated.load(Ordering::Relaxed) as f64;
        if o == 0.0 { 0.0 } else { n / o }
    }
}

pub fn get_current_memory_usage() -> u64 {
    use sysinfo::{Pid, ProcessesToUpdate, System};
    let pid = Pid::from(std::process::id() as usize);
    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::Some(&[pid]), false);
    sys.process(pid).map(|p: &sysinfo::Process| p.memory()).unwrap_or(0)
}

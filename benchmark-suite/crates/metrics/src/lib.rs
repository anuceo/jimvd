use serde::{Deserialize, Serialize};

/// Snapshot of all tracked measurements for one phase/interval.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Metrics {
    pub p50_latency_us:     u64,
    pub p95_latency_us:     u64,
    pub p99_latency_us:     u64,
    pub throughput_ops_sec: f64,
    pub storage_bytes:      u64,
    pub metadata_bytes:     u64,
    pub factor_utilization: f64,
    pub uaf:                f64,
    pub factor_count:       usize,
    pub graph_nodes:        usize,
    pub memory_peak_bytes:  u64,
}

/// Accumulates raw latency samples and computes percentiles.
#[derive(Debug, Clone, Default)]
pub struct LatencyHistogram {
    pub samples: Vec<u64>,
}

impl LatencyHistogram {
    pub fn new() -> Self {
        LatencyHistogram { samples: Vec::new() }
    }

    pub fn record(&mut self, d: std::time::Duration) {
        self.samples.push(d.as_micros() as u64);
    }

    /// Returns the pth percentile latency in microseconds.
    /// Sorts samples in place.
    pub fn percentile(&mut self, p: f64) -> u64 {
        if self.samples.is_empty() {
            return 0;
        }
        self.samples.sort_unstable();
        let idx = ((p / 100.0) * (self.samples.len() as f64 - 1.0)).round() as usize;
        self.samples[idx.min(self.samples.len() - 1)]
    }

    pub fn throughput(&self, total_duration: std::time::Duration) -> f64 {
        let secs = total_duration.as_secs_f64();
        if secs == 0.0 {
            0.0
        } else {
            self.samples.len() as f64 / secs
        }
    }

    pub fn count(&self) -> usize {
        self.samples.len()
    }
}

/// Update-Amplification-Factor tracker.
#[derive(Debug, Clone, Default)]
pub struct UafTracker {
    pub total_updates:       usize,
    pub total_nodes_touched: usize,
}

impl UafTracker {
    pub fn new() -> Self {
        UafTracker { total_updates: 0, total_nodes_touched: 0 }
    }

    pub fn record_update(&mut self, nodes_touched: usize) {
        self.total_updates += 1;
        self.total_nodes_touched += nodes_touched;
    }

    pub fn uaf(&self) -> f64 {
        if self.total_updates == 0 {
            0.0
        } else {
            self.total_nodes_touched as f64 / self.total_updates as f64
        }
    }

    pub fn reset(&mut self) {
        self.total_updates = 0;
        self.total_nodes_touched = 0;
    }
}

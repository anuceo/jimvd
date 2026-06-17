use std::time::Duration;
use anyhow::Result;

/// Results from executing a single operation.
#[derive(Debug)]
pub struct OpResult {
    pub latency:       Duration,
    pub nodes_touched: usize,
}

/// Every engine implements this trait.
pub trait DatabaseRunner {
    fn name(&self) -> &str;
    fn load_data(&mut self, users: &[data_generator::User]) -> Result<()>;
    fn load_table(&mut self, _table: &str, _users: &[data_generator::User]) -> Result<()> { Ok(()) }
    fn execute(&mut self, op: &workload_generator::Operation) -> Result<OpResult>;
    fn collect_metrics(&self) -> metrics::Metrics;
    fn reset_metrics(&mut self);
}

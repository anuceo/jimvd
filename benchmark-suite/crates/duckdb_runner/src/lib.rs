use anyhow::{anyhow, Result};
use runner_api::{DatabaseRunner, OpResult};
use workload_generator::{FilterPredicate, Operation};
#[cfg(feature = "duckdb")]
use std::time::Instant;

#[allow(dead_code)]
pub struct DuckdbRunner {
    #[cfg(feature = "duckdb")]
    conn: duckdb::Connection,
    latencies: metrics::LatencyHistogram,
}

impl DuckdbRunner {
    #[cfg(feature = "duckdb")]
    pub fn new() -> Result<Self> {
        let conn = duckdb::Connection::open_in_memory()?;
        Ok(DuckdbRunner { conn, latencies: metrics::LatencyHistogram::new() })
    }

    #[cfg(not(feature = "duckdb"))]
    pub fn new() -> Result<Self> {
        Err(anyhow!("duckdb feature not enabled"))
    }

    #[allow(dead_code)]
    fn pred_to_sql(pred: &FilterPredicate) -> String {
        match pred {
            FilterPredicate::Eq { attribute, value } => format!("{} = '{}'", attribute, value),
            FilterPredicate::And(subs) => {
                let inner: Vec<_> = subs.iter().map(Self::pred_to_sql).collect();
                format!("({})", inner.join(" AND "))
            }
            FilterPredicate::Or(subs) => {
                let inner: Vec<_> = subs.iter().map(Self::pred_to_sql).collect();
                format!("({})", inner.join(" OR "))
            }
        }
    }
}

impl DatabaseRunner for DuckdbRunner {
    fn name(&self) -> &str { "duckdb" }

    fn load_data(&mut self, users: &[data_generator::User]) -> Result<()> {
        #[cfg(feature = "duckdb")]
        {
            self.conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS users (
                    id BIGINT PRIMARY KEY,
                    tenant INTEGER,
                    department SMALLINT,
                    role SMALLINT,
                    region SMALLINT,
                    clearance SMALLINT,
                    manager BIGINT
                )",
            )?;
            for u in users {
                self.conn.execute(
                    "INSERT OR IGNORE INTO users VALUES (?,?,?,?,?,?,?)",
                    duckdb::params![
                        u.id as i64,
                        u.tenant as i32,
                        u.department as i16,
                        u.role as i16,
                        u.region as i16,
                        u.clearance as i16,
                        u.manager.map(|m| m as i64),
                    ],
                )?;
            }
            Ok(())
        }
        #[cfg(not(feature = "duckdb"))]
        {
            let _ = users;
            Err(anyhow!("duckdb feature not enabled"))
        }
    }

    fn execute(&mut self, op: &Operation) -> Result<OpResult> {
        #[cfg(feature = "duckdb")]
        {
            let start = Instant::now();
            match op {
                Operation::PointLookup { id } => {
                    self.conn.execute(
                        "SELECT * FROM users WHERE id = ?",
                        duckdb::params![*id as i64],
                    )?;
                }
                Operation::EqualityFilter { attribute, value } => {
                    let q = format!("SELECT * FROM users WHERE {} = '{}'", attribute, value);
                    self.conn.execute_batch(&q)?;
                }
                Operation::MultiAttributeFilter { predicates } => {
                    let where_clause: String = predicates.iter()
                        .map(|p| Self::pred_to_sql(p))
                        .collect::<Vec<_>>()
                        .join(" AND ");
                    let q = format!("SELECT * FROM users WHERE {}", where_clause);
                    self.conn.execute_batch(&q)?;
                }
                Operation::Update { user_id, attribute, new_value } => {
                    let q = format!(
                        "UPDATE users SET {} = '{}' WHERE id = {}",
                        attribute, new_value, user_id
                    );
                    self.conn.execute_batch(&q)?;
                }
                Operation::Delete { user_id } => {
                    self.conn.execute(
                        "DELETE FROM users WHERE id = ?",
                        duckdb::params![*user_id as i64],
                    )?;
                }
                Operation::Join { .. } => {
                    self.conn.execute_batch(
                        "SELECT u.id FROM users u JOIN users p ON u.role = p.role LIMIT 100",
                    )?;
                }
                Operation::Insert { user } => {
                    self.conn.execute(
                        "INSERT OR IGNORE INTO users VALUES (?,?,?,?,?,?,?)",
                        duckdb::params![
                            user.id as i64,
                            user.tenant as i32,
                            user.department as i16,
                            user.role as i16,
                            user.region as i16,
                            user.clearance as i16,
                            user.manager.map(|m| m as i64),
                        ],
                    )?;
                }
            }
            let latency = start.elapsed();
            self.latencies.record(latency);
            Ok(OpResult { latency, nodes_touched: 0 })
        }
        #[cfg(not(feature = "duckdb"))]
        {
            let _ = op;
            Err(anyhow!("duckdb feature not enabled"))
        }
    }

    fn collect_metrics(&self) -> metrics::Metrics {
        let mut h = self.latencies.clone();
        metrics::Metrics {
            p50_latency_us:     h.percentile(50.0),
            p95_latency_us:     h.percentile(95.0),
            p99_latency_us:     h.percentile(99.0),
            throughput_ops_sec: 0.0,
            ..Default::default()
        }
    }

    fn reset_metrics(&mut self) {
        self.latencies = metrics::LatencyHistogram::new();
    }
}

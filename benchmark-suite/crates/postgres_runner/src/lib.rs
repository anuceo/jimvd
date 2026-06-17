use anyhow::{anyhow, Result};
use runner_api::{DatabaseRunner, OpResult};
use workload_generator::{FilterPredicate, Operation};
#[cfg(feature = "postgres")]
use std::time::Instant;

#[allow(dead_code)]
pub struct PostgresRunner {
    #[cfg(feature = "postgres")]
    client: postgres::Client,
    latencies: metrics::LatencyHistogram,
}

impl PostgresRunner {
    #[cfg(feature = "postgres")]
    pub fn new(conn_str: &str) -> Result<Self> {
        let client = postgres::Client::connect(conn_str, postgres::NoTls)?;
        Ok(PostgresRunner { client, latencies: metrics::LatencyHistogram::new() })
    }

    #[cfg(not(feature = "postgres"))]
    pub fn new(_conn_str: &str) -> Result<Self> {
        Err(anyhow!("postgres feature not enabled"))
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

    #[allow(dead_code)]
    fn build_where_clause(predicates: &[FilterPredicate]) -> String {
        predicates.iter().map(|p| Self::pred_to_sql(p)).collect::<Vec<_>>().join(" AND ")
    }
}

impl DatabaseRunner for PostgresRunner {
    fn name(&self) -> &str { "postgres" }

    fn load_data(&mut self, users: &[data_generator::User]) -> Result<()> {
        #[cfg(feature = "postgres")]
        {
            self.client.execute(
                "CREATE TABLE IF NOT EXISTS users (
                    id BIGINT PRIMARY KEY,
                    tenant INT,
                    department SMALLINT,
                    role SMALLINT,
                    region SMALLINT,
                    clearance SMALLINT,
                    manager BIGINT
                )",
                &[],
            )?;
            for u in users {
                self.client.execute(
                    "INSERT INTO users (id,tenant,department,role,region,clearance,manager)
                     VALUES ($1,$2,$3,$4,$5,$6,$7) ON CONFLICT DO NOTHING",
                    &[
                        &(u.id as i64),
                        &(u.tenant as i32),
                        &(u.department as i16),
                        &(u.role as i16),
                        &(u.region as i16),
                        &(u.clearance as i16),
                        &u.manager.map(|m| m as i64),
                    ],
                )?;
            }
            Ok(())
        }
        #[cfg(not(feature = "postgres"))]
        {
            let _ = users;
            Err(anyhow!("postgres feature not enabled"))
        }
    }

    fn execute(&mut self, op: &Operation) -> Result<OpResult> {
        #[cfg(feature = "postgres")]
        {
            let start = Instant::now();
            match op {
                Operation::PointLookup { id } => {
                    self.client.query("SELECT * FROM users WHERE id = $1", &[&(*id as i64)])?;
                }
                Operation::EqualityFilter { attribute, value } => {
                    let q = format!("SELECT * FROM users WHERE {} = '{}'", attribute, value);
                    self.client.query(q.as_str(), &[])?;
                }
                Operation::MultiAttributeFilter { predicates } => {
                    let where_clause = Self::build_where_clause(predicates);
                    let q = format!("SELECT * FROM users WHERE {}", where_clause);
                    self.client.query(q.as_str(), &[])?;
                }
                Operation::Update { user_id, attribute, new_value } => {
                    let q = format!("UPDATE users SET {} = '{}' WHERE id = {}", attribute, new_value, user_id);
                    self.client.execute(q.as_str(), &[])?;
                }
                Operation::Delete { user_id } => {
                    self.client.execute("DELETE FROM users WHERE id = $1", &[&(*user_id as i64)])?;
                }
                Operation::Join { .. } => {
                    self.client.query(
                        "SELECT u.id FROM users u JOIN users p ON u.role = p.role LIMIT 100",
                        &[],
                    )?;
                }
                Operation::Insert { user } => {
                    self.client.execute(
                        "INSERT INTO users (id,tenant,department,role,region,clearance,manager)
                         VALUES ($1,$2,$3,$4,$5,$6,$7) ON CONFLICT DO NOTHING",
                        &[
                            &(user.id as i64),
                            &(user.tenant as i32),
                            &(user.department as i16),
                            &(user.role as i16),
                            &(user.region as i16),
                            &(user.clearance as i16),
                            &user.manager.map(|m| m as i64),
                        ],
                    )?;
                }
            }
            let latency = start.elapsed();
            self.latencies.record(latency);
            Ok(OpResult { latency, nodes_touched: 0 })
        }
        #[cfg(not(feature = "postgres"))]
        {
            let _ = op;
            Err(anyhow!("postgres feature not enabled"))
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

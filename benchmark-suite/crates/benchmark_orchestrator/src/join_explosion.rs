use runner_api::DatabaseRunner;
use workload_generator::Operation;

pub const JOIN_FANOUTS: &[usize] = &[1, 5, 20, 100, 500];

#[derive(Debug)]
pub struct JoinResult {
    pub avg_items_per_order: usize,
    pub runner_name:         String,
    pub metrics:             metrics::Metrics,
    pub fallback_rate:       f64,
}

pub fn run_join_explosion(
    runner:   &mut dyn DatabaseRunner,
    orders:   usize,
    products: usize,
    fanouts:  &[usize],
) -> Vec<JoinResult> {
    let mut results = Vec::new();

    for &fanout in fanouts {
        log::info!("JoinExplosion: fanout={}", fanout);
        runner.reset_metrics();

        // Generate synthetic rows using the data generator (reuse User struct as rows)
        let total_rows = (orders * fanout).max(1);
        let config = data_generator::CorrelationConfig::default();
        let users = data_generator::generate_users(total_rows, &config);

        if let Err(e) = runner.load_data(&users) {
            log::error!("load_data failed at fanout {}: {}", fanout, e);
            continue;
        }

        let join_count = orders.max(1);
        for _ in 0..join_count {
            let op = Operation::Join {
                left_table:     "users".to_string(),
                right_table:    "permissions".to_string(),
                join_attribute: "role".to_string(),
            };
            if let Err(e) = runner.execute(&op) {
                log::warn!("join execute error: {}", e);
            }
        }

        let _ = products; // suppress unused warning
        let metrics = runner.collect_metrics();
        let fallback_rate = 0.0;

        results.push(JoinResult {
            avg_items_per_order: fanout,
            runner_name:         runner.name().to_string(),
            metrics,
            fallback_rate,
        });
    }

    results
}

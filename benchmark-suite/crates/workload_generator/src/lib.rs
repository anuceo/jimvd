use serde::{Deserialize, Serialize};
use rand::RngExt;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FilterPredicate {
    Eq  { attribute: String, value: String },
    And(Vec<FilterPredicate>),
    Or (Vec<FilterPredicate>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Operation {
    PointLookup            { id: u64 },
    EqualityFilter         { attribute: String, value: String },
    MultiAttributeFilter   { predicates: Vec<FilterPredicate> },
    Insert                 { user: data_generator::User },
    Update                 { user_id: u64, attribute: String, new_value: String },
    Delete                 { user_id: u64 },
    Join                   { left_table: String, right_table: String, join_attribute: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkloadConfig {
    pub read_ratio:       f64,
    pub write_ratio:      f64,
    pub join_ratio:       f64,
    pub total_operations: usize,
    #[serde(default)]
    pub rng_seed:         u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Phase {
    IAM,
    Compliance,
    Tenant,
    Security,
    Custom(String),
}

pub struct WorkloadGenerator {
    config:       WorkloadConfig,
    phase:        Phase,
    dataset_size: usize,
    rng:          rand::rngs::ThreadRng,
}

impl WorkloadGenerator {
    pub fn new(config: WorkloadConfig, phase: Phase, dataset_size: usize) -> Self {
        WorkloadGenerator {
            config,
            phase,
            dataset_size,
            rng: rand::rng(),
        }
    }

    pub fn next_operation(&mut self) -> Operation {
        let r: f64 = self.rng.random();

        if r < self.config.join_ratio {
            return Operation::Join {
                left_table:     "users".to_string(),
                right_table:    "permissions".to_string(),
                join_attribute: "role".to_string(),
            };
        }

        let write_end = self.config.join_ratio + self.config.write_ratio;
        if r < write_end {
            return self.gen_write();
        }

        self.gen_read()
    }

    fn gen_read(&mut self) -> Operation {
        let ds = self.dataset_size.max(1);
        match self.phase.clone() {
            Phase::IAM => {
                let roles = ["Viewer", "Engineer", "Manager", "Admin"];
                let regions = ["US", "EU", "APAC"];
                let role_idx = self.rng.random_range(0..roles.len());
                let region_idx = self.rng.random_range(0..regions.len());
                let role = roles[role_idx];
                let region = regions[region_idx];
                if self.rng.random::<f64>() < 0.5 {
                    Operation::MultiAttributeFilter {
                        predicates: vec![
                            FilterPredicate::Eq { attribute: "role".to_string(),   value: role.to_string() },
                            FilterPredicate::Eq { attribute: "region".to_string(), value: region.to_string() },
                        ],
                    }
                } else {
                    let clearances = ["Public", "Confidential", "Secret", "TopSecret"];
                    let cl_idx = self.rng.random_range(0..clearances.len());
                    let cl = clearances[cl_idx];
                    Operation::MultiAttributeFilter {
                        predicates: vec![
                            FilterPredicate::Eq { attribute: "role".to_string(),      value: role.to_string() },
                            FilterPredicate::Eq { attribute: "region".to_string(),    value: region.to_string() },
                            FilterPredicate::Eq { attribute: "clearance".to_string(), value: cl.to_string() },
                        ],
                    }
                }
            }
            Phase::Compliance => {
                let regions = ["US", "EU", "APAC"];
                let policies = ["GDPR", "CCPA", "HIPAA", "SOX"];
                let region_idx = self.rng.random_range(0..regions.len());
                let policy_idx = self.rng.random_range(0..policies.len());
                let region = regions[region_idx];
                let policy = policies[policy_idx];
                Operation::MultiAttributeFilter {
                    predicates: vec![
                        FilterPredicate::Eq { attribute: "region".to_string(), value: region.to_string() },
                        FilterPredicate::Eq { attribute: "policy".to_string(), value: policy.to_string() },
                    ],
                }
            }
            Phase::Tenant => {
                let tenant_id = self.rng.random_range(0u32..100);
                let features = ["Billing", "Analytics", "Auth", "Storage"];
                let feature_idx = self.rng.random_range(0..features.len());
                let feature = features[feature_idx];
                Operation::MultiAttributeFilter {
                    predicates: vec![
                        FilterPredicate::Eq { attribute: "tenant".to_string(),  value: tenant_id.to_string() },
                        FilterPredicate::Eq { attribute: "feature".to_string(), value: feature.to_string() },
                    ],
                }
            }
            Phase::Security => {
                let clearances = ["Secret", "TopSecret"];
                let cl_idx = self.rng.random_range(0..clearances.len());
                let cl = clearances[cl_idx];
                Operation::EqualityFilter {
                    attribute: "clearance".to_string(),
                    value:     cl.to_string(),
                }
            }
            Phase::Custom(_) => {
                let id = self.rng.random_range(0..ds as u64);
                Operation::PointLookup { id }
            }
        }
    }

    fn gen_write(&mut self) -> Operation {
        let ds = self.dataset_size.max(1);
        let user_id = self.rng.random_range(0..ds as u64);
        if self.rng.random::<f64>() < 0.1 {
            return Operation::Delete { user_id };
        }
        if self.rng.random::<bool>() {
            let roles = ["Viewer", "Engineer", "Manager", "Admin"];
            let idx = self.rng.random_range(0..roles.len());
            let new_role = roles[idx];
            Operation::Update {
                user_id,
                attribute: "role".to_string(),
                new_value: new_role.to_string(),
            }
        } else {
            let regions = ["US", "EU", "APAC"];
            let idx = self.rng.random_range(0..regions.len());
            let new_region = regions[idx];
            Operation::Update {
                user_id,
                attribute: "region".to_string(),
                new_value: new_region.to_string(),
            }
        }
    }
}

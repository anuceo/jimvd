use serde::{Deserialize, Serialize};
use rand::RngExt;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id:         u64,
    pub tenant:     u32,
    pub department: u16,
    pub role:       u16,
    pub region:     u16,
    pub clearance:  u16,
    pub manager:    Option<u64>,
}

pub fn role_name(r: u16) -> &'static str {
    match r {
        1 => "Engineer",
        2 => "Manager",
        3 => "Admin",
        _ => "Viewer",
    }
}

pub fn region_name(r: u16) -> &'static str {
    match r {
        1 => "EU",
        2 => "APAC",
        _ => "US",
    }
}

pub fn dept_name(d: u16) -> &'static str {
    match d {
        1 => "Sales",
        2 => "HR",
        3 => "Finance",
        _ => "Engineering",
    }
}

pub fn clearance_name(c: u16) -> &'static str {
    match c {
        1 => "Confidential",
        2 => "Secret",
        3 => "TopSecret",
        _ => "Public",
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorrelationConfig {
    pub role_department_bias:  f64,
    pub region_clearance_bias: f64,
    pub tenant_role_bias:      f64,
}

impl Default for CorrelationConfig {
    fn default() -> Self {
        CorrelationConfig {
            role_department_bias:  0.7,
            region_clearance_bias: 0.5,
            tenant_role_bias:      0.8,
        }
    }
}

fn biased_choice(rng: &mut impl rand::Rng, count: usize, preferred: usize, bias: f64) -> usize {
    if rng.random::<f64>() < bias {
        preferred % count
    } else {
        rng.random_range(0..count)
    }
}

pub fn generate_users_seeded(count: usize, config: &CorrelationConfig, seed: u64) -> Vec<User> {
    use rand::SeedableRng;
    let mut rng = if seed == 0 {
        rand::rngs::StdRng::from_rng(&mut rand::rng())
    } else {
        rand::rngs::StdRng::seed_from_u64(seed)
    };
    let mut users = Vec::with_capacity(count);

    for i in 0..count {
        let tenant = rng.random_range(0u32..100);
        let role = biased_choice(
            &mut rng, 4,
            (tenant as usize * 3) % 4,
            config.tenant_role_bias,
        ) as u16;
        let department = biased_choice(
            &mut rng, 4,
            role as usize,
            config.role_department_bias,
        ) as u16;
        let region = rng.random_range(0u16..3);
        let clearance = biased_choice(
            &mut rng, 4,
            region as usize,
            config.region_clearance_bias,
        ) as u16;

        let manager = if role == 0 || i == 0 {
            None
        } else {
            Some(rng.random_range(0..i) as u64)
        };

        users.push(User {
            id: i as u64,
            tenant,
            department,
            role,
            region,
            clearance,
            manager,
        });
    }

    users
}

pub fn generate_users(count: usize, config: &CorrelationConfig) -> Vec<User> {
    let mut rng = rand::rng();
    let mut users = Vec::with_capacity(count);

    for i in 0..count {
        let tenant = rng.random_range(0u32..100);
        let role = biased_choice(
            &mut rng, 4,
            (tenant as usize * 3) % 4,
            config.tenant_role_bias,
        ) as u16;
        let department = biased_choice(
            &mut rng, 4,
            role as usize,
            config.role_department_bias,
        ) as u16;
        let region = rng.random_range(0u16..3);
        let clearance = biased_choice(
            &mut rng, 4,
            region as usize,
            config.region_clearance_bias,
        ) as u16;

        let manager = if role == 0 || i == 0 {
            None
        } else {
            Some(rng.random_range(0..i) as u64)
        };

        users.push(User {
            id: i as u64,
            tenant,
            department,
            role,
            region,
            clearance,
            manager,
        });
    }

    users
}

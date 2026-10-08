use super::settings::Limits;

pub fn eviction_target(limits: &Limits, used: u64) -> Option<u64> {
    if used < limits.budget {
        return None;
    }
    Some((limits.shrink_to * limits.budget as f64) as u64)
}

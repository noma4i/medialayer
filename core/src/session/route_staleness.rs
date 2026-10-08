use crate::proxy::Readiness;

pub struct RouteStaleness;

impl RouteStaleness {
    pub fn is_stale(
        supports_recovery: bool,
        resolved_generation: Option<u64>,
        current_generation: u64,
        readiness: Readiness,
    ) -> bool {
        if !supports_recovery {
            return false;
        }
        match resolved_generation {
            None => readiness == Readiness::Ready,
            Some(resolved) => resolved != current_generation,
        }
    }
}

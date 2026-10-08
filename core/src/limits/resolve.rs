use super::settings::{Limits, LimitsError};

pub fn resolve(defaults: Limits, saved: Option<&str>) -> (Limits, Option<LimitsError>) {
    let Some(text) = saved else {
        return (defaults, None);
    };
    match Limits::decode(defaults, text) {
        Ok(limits) => (limits, None),
        Err(error) => (defaults, Some(error)),
    }
}

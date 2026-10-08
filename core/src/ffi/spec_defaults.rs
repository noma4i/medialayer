use super::limits::{ImageWindow, Limits, PoolSettings};
use crate::api::{MediaError, PoolSettings as CorePoolSettings};
use crate::spec::MediaSpec;

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct SpecDefaults {
    pub defaults: Limits,
    pub pool: PoolSettings,
    pub namespace_headers: Vec<String>,
    #[uniffi(default = [])]
    pub image_windows: Vec<ImageWindow>,
}

#[uniffi::export]
pub fn spec_defaults(json: String) -> Result<SpecDefaults, MediaError> {
    let invalid = |error: &dyn std::fmt::Display| MediaError::InvalidSpec { detail: error.to_string() };
    let spec = MediaSpec::from_json(&json).map_err(|error| invalid(&error))?;
    spec.validate().map_err(|error| invalid(&error))?;
    Ok(SpecDefaults {
        defaults: spec.limits().into(),
        pool: CorePoolSettings::from(&spec.pool()).into(),
        image_windows: spec
            .images
            .iter()
            .filter_map(|image| image.window().map(|(behind, from, to)| ImageWindow { name: image.name.clone(), behind, from, to }))
            .collect(),
        namespace_headers: spec.cache.namespace_headers,
    })
}

use std::collections::HashSet;

use crate::limits::LimitsError;
use crate::spec::MediaSpec;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SpecError {
    #[error("`{0}` is declared twice")]
    DuplicateName(String),
    #[error("`{name}` has a zero width or height")]
    ZeroImageSize { name: String },
    #[error("`{name}` has a prefetch window that starts after it ends")]
    WindowInverted { name: String },
    #[error("cache: {0}")]
    Cache(LimitsError),
    #[error("video `{name}`: {error}")]
    Video { name: String, error: LimitsError },
}

pub fn type_name(name: &str) -> String {
    name.split('_')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut characters = part.chars();
            characters.next().map(|first| first.to_uppercase().chain(characters).collect::<String>()).unwrap_or_default()
        })
        .collect()
}

impl MediaSpec {
    pub fn validate(&self) -> Result<(), SpecError> {
        self.check_names()?;
        self.check_images()?;
        self.limits_for(None).validate().map_err(SpecError::Cache)?;
        for video in &self.videos {
            self.limits_for(Some(video))
                .validate()
                .map_err(|error| SpecError::Video { name: video.name.clone(), error })?;
        }
        Ok(())
    }

    fn check_names(&self) -> Result<(), SpecError> {
        let names = self
            .videos
            .iter()
            .map(|video| &video.name)
            .chain(self.images.iter().map(|image| &image.name))
            .chain(self.audios.iter().map(|audio| &audio.name));
        let mut seen = HashSet::new();
        for name in names {
            if !seen.insert(type_name(name)) {
                return Err(SpecError::DuplicateName(name.clone()));
            }
        }
        Ok(())
    }

    fn check_images(&self) -> Result<(), SpecError> {
        for image in &self.images {
            if image.size.is_some_and(|(width, height)| width == 0 || height == 0) {
                return Err(SpecError::ZeroImageSize { name: image.name.clone() });
            }
            if image.prefetch_window.is_some_and(|(from, to)| from > to) {
                return Err(SpecError::WindowInverted { name: image.name.clone() });
            }
        }
        Ok(())
    }
}

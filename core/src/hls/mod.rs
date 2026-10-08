mod leading_segments;
mod manifest_url;
mod rewrite;
mod startup_error;

pub use leading_segments::{
    extract_leading_segments, extract_variants, is_multivariant, resolve_url, LeadingSegments,
};
pub use manifest_url::is_manifest_url;
pub use rewrite::{restore_rewritten, rewrite_playlist};
pub use startup_error::{describe, is_fatal_startup_http_error};

#[cfg(test)]
mod leading_segments_tests;
#[cfg(test)]
mod manifest_url_tests;
#[cfg(test)]
mod rewrite_tests;
#[cfg(test)]
mod startup_error_tests;

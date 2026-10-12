mod leading_chunks;
mod manifest_url;
mod rewrite;
mod single_variant;
mod startup_error;

pub use leading_chunks::{extract_leading_chunks, extract_variants, is_multivariant, resolve_url, Chunk};
pub use manifest_url::is_manifest_url;
pub use rewrite::{restore_rewritten, rewrite_playlist};
pub use single_variant::keep_first_variant;
pub use startup_error::{describe, is_fatal_startup_http_error};

#[cfg(test)]
mod leading_chunks_tests;
#[cfg(test)]
mod manifest_url_tests;
#[cfg(test)]
mod rewrite_tests;
#[cfg(test)]
mod startup_error_tests;

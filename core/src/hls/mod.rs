mod leading_chunks;
mod manifest_url;
mod rewrite;
mod startup_error;
mod playlist;
pub use playlist::Playlist;

#[cfg(test)]
pub(crate) mod work {
    use std::cell::Cell;
    thread_local! {
        pub static URLS: Cell<usize> = const { Cell::new(0) };
        pub static PASSES: Cell<usize> = const { Cell::new(0) };
    }
    pub fn url() { URLS.with(|value| value.set(value.get() + 1)); }
    pub fn pass() { PASSES.with(|value| value.set(value.get() + 1)); }
}

pub use leading_chunks::Chunk;
pub use manifest_url::is_manifest_url;
pub use rewrite::restore_rewritten;
pub use startup_error::{describe, is_fatal_startup_http_error};

#[cfg(test)]
mod leading_chunks_tests;
#[cfg(test)]
mod manifest_url_tests;
#[cfg(test)]
mod rewrite_tests;
#[cfg(test)]
mod startup_error_tests;

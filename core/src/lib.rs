uniffi::setup_scaffolding!();

pub mod analysis;
pub mod api;
pub mod blurhash;
pub mod cache;
pub mod change;
pub mod fetch;
pub mod ffi;
pub mod hls;
pub mod identity;
pub mod limits;
pub mod memory;
pub mod pages;
pub mod pool;
pub mod prefetch;
pub mod proxy;
pub mod session;
pub mod spec;
pub mod trace;

#[uniffi::export]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[cfg(test)]
mod alloc_probe;
#[cfg(test)]
#[global_allocator]
static ALLOCATOR: alloc_probe::Counting = alloc_probe::Counting;
#[cfg(test)]
mod hardening_tests;
#[cfg(test)]
mod seeded;
#[cfg(test)]
mod lib_tests;

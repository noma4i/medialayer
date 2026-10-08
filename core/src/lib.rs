uniffi::setup_scaffolding!();

pub mod hls;
pub mod identity;
pub mod limits;
pub mod pool;
pub mod prefetch;

#[uniffi::export]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[cfg(test)]
mod lib_tests;

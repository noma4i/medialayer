mod fetcher;
mod policy;

#[cfg(test)]
pub mod mock;

pub use fetcher::{header_value, BodyStream, FetchError, Fetcher, Request, Response};
pub use policy::{FetchPolicy, DEFAULT_FORWARDED_HEADERS};

#[cfg(test)]
mod policy_tests;

use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

static ENABLED: OnceLock<bool> = OnceLock::new();

pub fn enabled() -> bool {
    *ENABLED.get_or_init(|| std::env::var_os("ML_TRACE").is_some())
}

pub fn now_ms() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |elapsed| elapsed.as_millis())
}

pub fn tail(url: &str) -> &str {
    let start = url.char_indices().rev().nth(23).map_or(0, |(index, _)| index);
    &url[start..]
}

#[macro_export]
macro_rules! trace {
    ($source:expr, $($argument:tt)*) => {
        if $crate::trace::enabled() {
            eprintln!("ML {} {} {}", $crate::trace::now_ms(), $source, format!($($argument)*));
        }
    };
}

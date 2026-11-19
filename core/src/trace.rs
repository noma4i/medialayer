use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

static ENV: OnceLock<bool> = OnceLock::new();
static SINK_ON: AtomicBool = AtomicBool::new(false);
static SINK: RwLock<Option<Arc<dyn TraceSink>>> = RwLock::new(None);

/// Takes the trace lines of the core where the process has no stderr to read, such as an Android app.
#[uniffi::export(with_foreign)]
pub trait TraceSink: Send + Sync {
    fn line(&self, text: String);
}

/// Routes every trace line to the sink and turns the trace on, or removes the sink.
#[uniffi::export]
pub fn set_trace_sink(sink: Option<Arc<dyn TraceSink>>) {
    SINK_ON.store(sink.is_some(), Ordering::Release);
    *SINK.write().unwrap_or_else(std::sync::PoisonError::into_inner) = sink;
}

/// Held by a test for as long as it owns the process-wide sink: two tests that install a sink would overwrite each other.
#[cfg(test)]
pub(crate) fn sink_test_guard() -> std::sync::MutexGuard<'static, ()> {
    static OWNER: std::sync::Mutex<()> = std::sync::Mutex::new(());
    OWNER.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub fn enabled() -> bool {
    SINK_ON.load(Ordering::Acquire) || *ENV.get_or_init(|| std::env::var_os("ML_TRACE").is_some())
}

pub fn now_ms() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |elapsed| elapsed.as_millis())
}

/// The last 24 characters of the url path: the query and the fragment, where a token or a signature sits, are cut first.
pub fn tail(url: &str) -> &str {
    let url = url.split(['?', '#']).next().unwrap_or(url);
    let start = url.char_indices().rev().nth(23).map_or(0, |(index, _)| index);
    &url[start..]
}

pub fn emit(source: &str, text: &str) {
    let line = format!("ML {} {source} {text}", now_ms());
    let sink = SINK.read().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
    match sink {
        Some(sink) => sink.line(line),
        None => eprintln!("{line}"),
    }
}

#[macro_export]
macro_rules! trace {
    ($source:expr, $($argument:tt)*) => {
        if $crate::trace::enabled() {
            $crate::trace::emit(&$source.to_string(), &format!($($argument)*));
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct Lines(Mutex<Vec<String>>);

    impl TraceSink for Lines {
        fn line(&self, text: String) {
            self.0.lock().unwrap().push(text);
        }
    }

    #[test]
    fn tail_cuts_query_and_fragment_before_taking_the_end() {
        assert_eq!(tail("https://cdn.example.com/media/clip.mp4?token=SECRETSECRETSECRETSECRET#frag"), "/media/clip.mp4");
        assert_eq!(tail("https://cdn.example.com/a/very/long/path/to/clip.mp4"), "/long/path/to/clip.mp4");
        assert_eq!(tail("?token=abc"), "");
    }

    #[test]
    fn inv_trace_01_an_installed_sink_turns_the_trace_on_and_takes_the_lines_without_the_environment() {
        let _owner = sink_test_guard();
        let lines = Arc::new(Lines(Mutex::default()));

        set_trace_sink(Some(lines.clone()));
        crate::trace!("probe", "inv_trace_01 slot={}", 4);
        let seen: Vec<String> = lines.0.lock().unwrap().iter().filter(|line| line.contains("inv_trace_01")).cloned().collect();
        set_trace_sink(None);

        assert_eq!(seen.len(), 1);
        let parts: Vec<&str> = seen[0].splitn(4, ' ').collect();
        assert_eq!((parts[0], parts[2], parts[3]), ("ML", "probe", "inv_trace_01 slot=4"));
        assert!(parts[1].parse::<u128>().is_ok());
    }
}

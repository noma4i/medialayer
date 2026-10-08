pub fn is_fatal_startup_http_error(status_code: i32, has_loaded: bool) -> bool {
    status_code >= 400 && !has_loaded
}

pub fn describe(status_code: i32, comment: Option<&str>) -> String {
    match comment {
        Some(text) if !text.is_empty() => format!("HLS load failed: HTTP {status_code} - {text}"),
        _ => format!("HLS load failed: HTTP {status_code}"),
    }
}

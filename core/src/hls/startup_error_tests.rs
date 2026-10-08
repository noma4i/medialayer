use super::*;

#[test]
fn http_error_is_fatal_before_first_load() {
    assert!(is_fatal_startup_http_error(401, false));
    assert!(is_fatal_startup_http_error(400, false));
    assert!(is_fatal_startup_http_error(503, false));
}

#[test]
fn http_error_is_not_fatal_after_first_load() {
    assert!(!is_fatal_startup_http_error(503, true));
}

#[test]
fn non_failing_status_is_not_fatal() {
    assert!(!is_fatal_startup_http_error(0, false));
    assert!(!is_fatal_startup_http_error(200, false));
    assert!(!is_fatal_startup_http_error(399, false));
}

#[test]
fn describe_includes_a_non_empty_comment() {
    assert_eq!(describe(503, Some("busy")), "HLS load failed: HTTP 503 - busy");
}

#[test]
fn describe_skips_a_missing_or_empty_comment() {
    assert_eq!(describe(404, None), "HLS load failed: HTTP 404");
    assert_eq!(describe(404, Some("")), "HLS load failed: HTTP 404");
}

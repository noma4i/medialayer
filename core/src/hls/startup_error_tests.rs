use super::*;

#[test]
fn inv_hls_06_http_error_is_fatal_only_before_the_first_load_and_from_status_400() {
    for (status, has_loaded, fatal) in [
        (400, false, true),
        (401, false, true),
        (503, false, true),
        (503, true, false),
        (399, false, false),
        (200, false, false),
        (0, false, false),
    ] {
        assert_eq!(is_fatal_startup_http_error(status, has_loaded), fatal, "{status} loaded={has_loaded}");
    }
}

#[test]
fn inv_hls_06_describe_adds_the_comment_only_when_it_is_not_empty() {
    assert_eq!(describe(503, Some("busy")), "HLS load failed: HTTP 503 - busy");
    assert_eq!(describe(404, None), "HLS load failed: HTTP 404");
    assert_eq!(describe(404, Some("")), "HLS load failed: HTTP 404");
}

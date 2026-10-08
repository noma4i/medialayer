use super::*;

#[test]
fn inv_session_10_stale_only_when_recoverable_and_port_changed_or_direct_route_faces_a_ready_proxy() {
    use crate::proxy::Readiness::*;
    assert!(!RouteStaleness::is_stale(false, Some(1), 2, Ready));
    assert!(!RouteStaleness::is_stale(false, None, 2, Ready));
    assert!(RouteStaleness::is_stale(true, Some(1), 2, Ready));
    assert!(!RouteStaleness::is_stale(true, Some(2), 2, Unhealthy));
    assert!(RouteStaleness::is_stale(true, None, 0, Ready));
    assert!(!RouteStaleness::is_stale(true, None, 0, Unhealthy));
    assert!(!RouteStaleness::is_stale(true, None, 0, Stopped));
    for readiness in [Stopped, Ready, Unhealthy] {
        assert!(RouteStaleness::is_stale(true, Some(7), 8, readiness));
    }
}

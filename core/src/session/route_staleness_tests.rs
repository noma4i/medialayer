use super::*;

#[test]
fn stale_only_when_recoverable_and_port_changed_or_direct_route_faces_a_ready_proxy() {
    use Readiness::*;
    assert!(!RouteStaleness::is_stale(false, Some(1), 2, Ready));
    assert!(!RouteStaleness::is_stale(false, None, 2, Ready));
    assert!(RouteStaleness::is_stale(true, Some(1), 2, Ready));
    assert!(!RouteStaleness::is_stale(true, Some(2), 2, Unhealthy));
    assert!(RouteStaleness::is_stale(true, None, 0, Ready));
    assert!(!RouteStaleness::is_stale(true, None, 0, Unhealthy));
    assert!(!RouteStaleness::is_stale(true, None, 0, Stopped));
}

#[test]
fn changed_generation_is_stale_whatever_the_readiness() {
    for readiness in [Readiness::Stopped, Readiness::Ready, Readiness::Unhealthy] {
        assert!(RouteStaleness::is_stale(true, Some(7), 8, readiness));
    }
}

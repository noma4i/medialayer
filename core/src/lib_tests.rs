use super::*;

#[test]
fn version_comes_from_the_crate() {
    assert_eq!(version(), "2.1.0");
}

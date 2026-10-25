use super::layer_life::{LayerLife, LayerPhase};

#[test]
fn inv_ffi_17_a_result_that_arrives_after_the_shutdown_is_not_installed() {
    let mut late = LayerLife::new();
    assert!(late.shutdown());
    assert!(!late.installed() && !late.failed(), "a late assembly result moved a closing layer");
    assert_eq!(late.phase(), LayerPhase::Closing);

    let mut ready = LayerLife::new();
    assert!(ready.installed());
    assert!(ready.shutdown() && !ready.shutdown(), "the close started twice");
    assert!(ready.cleaned());
    assert_eq!(ready.phase(), LayerPhase::Closed);
    assert!(!ready.installed() && !ready.shutdown());
}

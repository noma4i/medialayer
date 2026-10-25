use super::*;
use std::sync::atomic::Ordering;

fn verified(segments: usize) -> Arc<Verified> {
    Verified::loaded(&Check(Arc::from(vec![0u64; segments])))
}

#[test]
fn inv_cache_33_marking_is_linear_for_forward_reverse_and_duplicate_orders() {
    const SEGMENTS: usize = 512;
    let orders: [(&str, Vec<usize>); 3] = [
        ("forward", (0..SEGMENTS).collect()),
        ("reverse", (0..SEGMENTS).rev().collect()),
        ("duplicate", (0..SEGMENTS).flat_map(|index| [index, index]).collect()),
    ];
    for (name, order) in orders {
        let verified = verified(SEGMENTS);
        for index in &order {
            verified.mark(*index);
        }
        assert!(verified.is_complete(), "{name}");
        assert!(verified.cells.load(Ordering::Relaxed) <= SEGMENTS + order.len(), "{name}: marking examined more cells than one per call plus the bitmap");
    }
}

#[test]
fn inv_cache_33_completion_follows_the_last_distinct_segment_and_frees_the_bitmap() {
    let verified = verified(64);
    for _ in 0..200 {
        verified.mark(0);
    }
    assert!(!verified.is_complete());
    for index in 1..63 {
        verified.mark(index);
    }
    assert!(!verified.is_complete());

    verified.mark(63);

    let freed = |verified: &Verified| verified.progress.lock().unwrap().is_none();
    assert!(verified.is_complete());
    assert!(freed(&verified));
    verified.mark(5);
    assert!(verified.is_complete());
    assert!(freed(&verified));
    assert!(verified.missing(0, 63).is_empty());
}

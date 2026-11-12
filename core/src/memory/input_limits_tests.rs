use super::*;

#[test]
fn inv_memory_07_a_length_at_the_limit_passes_and_one_more_fails() {
    assert_eq!(bounded(10, 10), Ok(()));
    assert_eq!(bounded(11, 10), Err(TooLarge { len: 11, limit: 10 }));
}

use super::*;

const V3_WINDOW: PrefetchWindow = PrefetchWindow { from: 2, to: 5 };

fn keys(count: usize) -> Vec<Option<String>> {
    (0..count).map(|index| Some(format!("m{index}"))).collect()
}

fn names(jobs: &[PrefetchJob]) -> Vec<&str> {
    jobs.iter().map(|job| job.key.as_str()).collect()
}

fn priorities(jobs: &[PrefetchJob]) -> Vec<i64> {
    jobs.iter().map(|job| job.priority).collect()
}

#[test]
fn inv_prefetch_05_active_zero_wants_pages_two_to_five_with_index_minus_active_priority() {
    let wanted = wanted_jobs(0, &keys(20), V3_WINDOW);
    assert_eq!(names(&wanted), ["m2", "m3", "m4", "m5"]);
    assert_eq!(priorities(&wanted), [2, 3, 4, 5]);
}

#[test]
fn inv_prefetch_05_first_diff_starts_everything_and_cancels_nothing() {
    let wanted = wanted_jobs(0, &keys(20), V3_WINDOW);
    let diff = diff_window(&[], &wanted);
    assert_eq!(names(&diff.to_start), ["m2", "m3", "m4", "m5"]);
    assert!(diff.to_cancel.is_empty());
}

#[test]
fn inv_prefetch_05_shift_by_one_cancels_page_two_and_starts_page_six() {
    let before = wanted_jobs(0, &keys(20), V3_WINDOW);
    let after = wanted_jobs(1, &keys(20), V3_WINDOW);
    let diff = diff_window(&before, &after);
    assert_eq!(names(&diff.to_cancel), ["m2"]);
    assert_eq!(names(&diff.to_start), ["m6"]);
    assert_eq!(names(&after), ["m3", "m4", "m5", "m6"]);
    assert_eq!(priorities(&after), [2, 3, 4, 5]);
}

#[test]
fn inv_prefetch_05_same_window_gives_empty_diff() {
    let wanted = wanted_jobs(1, &keys(20), V3_WINDOW);
    let diff = diff_window(&wanted, &wanted);
    assert!(diff.to_start.is_empty());
    assert!(diff.to_cancel.is_empty());
}

#[test]
fn inv_prefetch_05_list_change_cancels_leaving_and_starts_entering() {
    let before = wanted_jobs(0, &keys(20), V3_WINDOW);
    let shifted: Vec<Option<String>> = keys(20).into_iter().skip(1).collect();
    let after = wanted_jobs(0, &shifted, V3_WINDOW);
    let diff = diff_window(&before, &after);
    assert_eq!(names(&diff.to_cancel), ["m2"]);
    assert_eq!(names(&after), ["m3", "m4", "m5", "m6"]);
    assert_eq!(names(&diff.to_start), ["m6"]);
}

#[test]
fn inv_prefetch_05_empty_wanted_cancels_everything() {
    let before = wanted_jobs(0, &keys(20), V3_WINDOW);
    let diff = diff_window(&before, &[]);
    assert_eq!(names(&diff.to_cancel), ["m2", "m3", "m4", "m5"]);
    assert!(diff.to_start.is_empty());
}

#[test]
fn inv_prefetch_05_pages_without_source_are_not_wanted() {
    let mut list = keys(20);
    list[3] = None;
    assert_eq!(names(&wanted_jobs(0, &list, V3_WINDOW)), ["m2", "m4", "m5"]);
}

#[test]
fn inv_prefetch_05_window_is_clamped_to_the_list() {
    assert_eq!(names(&wanted_jobs(17, &keys(20), V3_WINDOW)), ["m19"]);
    assert!(wanted_jobs(18,&keys(20), V3_WINDOW).is_empty());
    assert!(wanted_jobs(0, &[], V3_WINDOW).is_empty());
    assert!(wanted_jobs(25, &keys(20), V3_WINDOW).is_empty());
}

#[test]
fn inv_prefetch_05_window_is_a_parameter() {
    let wanted = wanted_jobs(5, &keys(20), PrefetchWindow { from: 1, to: 1 });
    assert_eq!(names(&wanted), ["m6"]);
    assert_eq!(priorities(&wanted), [1]);
}

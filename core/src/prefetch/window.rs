use crate::pages::{Keyed, PageKeys, Sliced};
use std::collections::HashSet;
use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrefetchWindow {
    pub from: usize,
    pub to: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageWindow {
    pub behind: usize,
    pub from: usize,
    pub to: usize,
}

impl PrefetchWindow {
    pub fn range(&self, active: usize, count: usize) -> Range<usize> {
        active.saturating_add(self.from).min(count)..active.saturating_add(self.to).saturating_add(1).min(count)
    }
}

impl PageWindow {
    pub const DEFAULT: PageWindow = PageWindow { behind: 2, from: 0, to: 9 };

    pub fn range(&self, active: usize, count: usize) -> Range<usize> {
        active.saturating_sub(self.behind).min(count)..active.saturating_add(self.to).min(count)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrefetchJob {
    pub key: String,
    pub priority: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WindowDiff {
    pub to_start: Vec<PrefetchJob>,
    pub to_cancel: Vec<PrefetchJob>,
}

pub fn wanted_jobs(active: usize, keys: &(impl PageKeys + ?Sized), window: PrefetchWindow) -> Vec<PrefetchJob> {
    if active >= keys.count() {
        return Vec::new();
    }
    window
        .range(active, keys.count())
        .filter_map(|index| keys.key(index).map(|key| PrefetchJob { key: key.to_string(), priority: index as i64 - active as i64 }))
        .collect()
}

pub fn wanted_page_jobs<S: Keyed>(active: usize, pages: &Sliced<Vec<S>>, window: PageWindow) -> Vec<PrefetchJob> {
    if active >= pages.count() {
        return Vec::new();
    }
    let ahead_start = active.saturating_add(window.from).min(pages.count());
    let ahead_end = active.saturating_add(window.to).min(pages.count());
    let behind_start = active.saturating_sub(window.behind);
    let order = (ahead_start..ahead_end).chain((behind_start..active).rev());
    let mut seen = HashSet::new();
    let mut jobs = Vec::new();
    for page in order {
        for key in pages.get(page).into_iter().flatten().map(Keyed::key) {
            if seen.insert(key) {
                jobs.push(PrefetchJob { key: key.to_string(), priority: page as i64 - active as i64 });
            }
        }
    }
    jobs
}

pub fn diff_window(previous_jobs: &[PrefetchJob], wanted: &[PrefetchJob]) -> WindowDiff {
    let wanted_keys: HashSet<&str> = wanted.iter().map(|job| job.key.as_str()).collect();
    let running_keys: HashSet<&str> = previous_jobs.iter().map(|job| job.key.as_str()).collect();
    WindowDiff {
        to_start: wanted.iter().filter(|job| !running_keys.contains(job.key.as_str())).cloned().collect(),
        to_cancel: previous_jobs.iter().filter(|job| !wanted_keys.contains(job.key.as_str())).cloned().collect(),
    }
}

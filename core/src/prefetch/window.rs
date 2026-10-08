use std::collections::HashSet;

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

impl PageWindow {
    pub const DEFAULT: PageWindow = PageWindow { behind: 2, from: 0, to: 9 };
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

pub fn wanted_jobs(active: usize, keys: &[Option<String>], window: PrefetchWindow) -> Vec<PrefetchJob> {
    if active >= keys.len() {
        return Vec::new();
    }
    let lower = active.saturating_add(window.from).min(keys.len());
    let upper = active.saturating_add(window.to).saturating_add(1).min(keys.len());
    (lower..upper)
        .filter_map(|index| {
            keys[index].as_ref().map(|key| PrefetchJob { key: key.clone(), priority: index as i64 - active as i64 })
        })
        .collect()
}

pub fn wanted_page_jobs(active: usize, pages: &[Vec<String>], window: PageWindow) -> Vec<PrefetchJob> {
    if active >= pages.len() {
        return Vec::new();
    }
    let ahead_start = active.saturating_add(window.from).min(pages.len());
    let ahead_end = active.saturating_add(window.to).min(pages.len());
    let behind_start = active.saturating_sub(window.behind);
    let order = (ahead_start..ahead_end).chain((behind_start..active).rev());
    let mut seen = HashSet::new();
    let mut jobs = Vec::new();
    for page in order {
        for key in &pages[page] {
            if seen.insert(key.as_str()) {
                jobs.push(PrefetchJob { key: key.clone(), priority: page as i64 - active as i64 });
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

use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrefetchWindow {
    pub from: usize,
    pub to: usize,
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

pub fn diff_window(previous_jobs: &[PrefetchJob], wanted: &[PrefetchJob]) -> WindowDiff {
    let wanted_keys: HashSet<&str> = wanted.iter().map(|job| job.key.as_str()).collect();
    let running_keys: HashSet<&str> = previous_jobs.iter().map(|job| job.key.as_str()).collect();
    WindowDiff {
        to_start: wanted.iter().filter(|job| !running_keys.contains(job.key.as_str())).cloned().collect(),
        to_cancel: previous_jobs.iter().filter(|job| !wanted_keys.contains(job.key.as_str())).cloned().collect(),
    }
}

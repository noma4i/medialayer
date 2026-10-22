use super::inner::{locked, ProxyShared};
use super::recent::Recent;
use crate::limits::MAX_CHUNKS_AHEAD;
use crate::pool::Retention;
use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Notify;
use tokio::time::Instant;

const MAX_STREAMS: usize = 16;
const WARM_CHUNKS_AHEAD: usize = 1;
const RECHECK_MIN: Duration = Duration::from_millis(100);
const RECHECK_MAX: Duration = Duration::from_secs(1);
const FULL_PERCENT: u32 = 100;

pub type PositionProbe = Pin<Box<dyn Future<Output = Option<u64>> + Send>>;

pub trait Playhead: Send + Sync {
    fn position_ms(&self, playlist: &str) -> PositionProbe;
    fn level(&self, playlist: &str) -> Option<Retention>;
    fn readahead(&self, playlist: &str, hint_ms: u64);
}

pub(super) struct Stream {
    root: String,
    starts_ms: Vec<u64>,
    target_ms: u64,
    grown: AtomicU32,
    nudge: Notify,
}

impl Stream {
    fn index_at(&self, position_ms: u64) -> usize {
        self.starts_ms.partition_point(|start| *start <= position_ms).saturating_sub(1)
    }
}

struct Table {
    by_segment: HashMap<String, (Arc<Stream>, usize)>,
    by_root: HashMap<String, Arc<Stream>>,
    order: VecDeque<String>,
    variants: Recent<String>,
}

impl Table {
    fn new() -> Self {
        Table { by_segment: HashMap::new(), by_root: HashMap::new(), order: VecDeque::new(), variants: Recent::new(MAX_STREAMS * 2) }
    }
}

pub(super) struct Windows {
    table: Mutex<Table>,
    playhead: Mutex<Option<Arc<dyn Playhead>>>,
    chunks_ahead: AtomicU32,
    scale_percent: AtomicU32,
}

impl Windows {
    pub fn new(chunks_ahead: u32) -> Self {
        Windows {
            table: Mutex::new(Table::new()),
            playhead: Mutex::new(None),
            chunks_ahead: AtomicU32::new(chunks_ahead),
            scale_percent: AtomicU32::new(FULL_PERCENT),
        }
    }

    pub fn set_chunks_ahead(&self, chunks_ahead: u32) {
        self.chunks_ahead.store(chunks_ahead, Ordering::SeqCst);
    }

    pub fn set_scale_percent(&self, percent: u32) {
        self.scale_percent.store(percent, Ordering::SeqCst);
        let streams: Vec<Arc<Stream>> = locked(&self.table).by_root.values().cloned().collect();
        for stream in streams {
            stream.nudge.notify_waiters();
            self.announce(&stream);
        }
    }

    pub fn set_playhead(&self, playhead: Arc<dyn Playhead>) {
        *locked(&self.playhead) = Some(playhead);
    }

    fn playhead(&self) -> Option<Arc<dyn Playhead>> {
        locked(&self.playhead).clone()
    }

    fn cap(&self, stream: &Stream) -> usize {
        let base = (self.chunks_ahead.load(Ordering::SeqCst) * self.scale_percent.load(Ordering::SeqCst) / FULL_PERCENT).max(1);
        (base + stream.grown.load(Ordering::SeqCst)).min(MAX_CHUNKS_AHEAD) as usize
    }

    fn ahead(&self, stream: &Stream) -> usize {
        match self.playhead().and_then(|playhead| playhead.level(&stream.root)) {
            None | Some(Retention::Hot) => self.cap(stream),
            Some(Retention::Warm | Retention::Cold) => WARM_CHUNKS_AHEAD,
        }
    }

    fn hint_ms(&self, stream: &Stream) -> u64 {
        self.ahead(stream).max(1) as u64 * stream.target_ms
    }

    pub fn hint_for(&self, root: &str) -> Option<u64> {
        let stream = locked(&self.table).by_root.get(root).cloned()?;
        Some(self.hint_ms(&stream))
    }

    pub fn remember_variant(&self, variant: &str, root: &str) {
        locked(&self.table).variants.insert(variant, root.to_string(), |_, _| false);
    }

    #[cfg(test)]
    pub fn variant_count(&self) -> usize {
        locked(&self.table).variants.len()
    }

    #[cfg(test)]
    pub fn variant_root(&self, variant: &str) -> Option<String> {
        locked(&self.table).variants.get(variant).cloned()
    }

    pub fn register_plan(&self, playlist: &str, plan: &crate::hls::Playlist) {
        if !plan.is_vod { return; }
        let segments = plan.chunks.iter().zip(&plan.durations).map(|(chunk, duration)| (chunk.segment.clone(), *duration)).collect();
        self.register_segments(playlist, segments, plan.target_ms);
    }

    fn register_segments(&self, playlist: &str, segments: Vec<(String, u64)>, target_ms: u64) {
        let mut starts_ms = Vec::with_capacity(segments.len());
        let mut elapsed = 0;
        for (_, duration) in &segments {
            starts_ms.push(elapsed);
            elapsed += duration;
        }
        let root = locked(&self.table).variants.get(playlist).cloned().unwrap_or_else(|| playlist.to_string());
        let stream = Arc::new(Stream { root: root.clone(), starts_ms, target_ms, grown: AtomicU32::new(0), nudge: Notify::new() });
        {
            let mut table = locked(&self.table);
            Self::forget(&mut table, &root);
            for (index, (url, _)) in segments.into_iter().enumerate() {
                table.by_segment.insert(url, (stream.clone(), index));
            }
            table.by_root.insert(root.clone(), stream.clone());
            table.order.push_back(root.clone());
            while table.order.len() > MAX_STREAMS {
                let Some(oldest) = table.order.pop_front() else { break };
                Self::forget(&mut table, &oldest);
            }
        }
        self.announce(&stream);
    }

    fn forget(table: &mut Table, root: &str) {
        let Some(stream) = table.by_root.remove(root) else { return };
        table.by_segment.retain(|_, (owner, _)| !Arc::ptr_eq(owner, &stream));
        table.order.retain(|queued| queued != root);
    }

    fn announce(&self, stream: &Stream) {
        if let Some(playhead) = self.playhead() {
            playhead.readahead(&stream.root, self.hint_ms(stream));
        }
    }

    pub fn is_planned(&self, url: &str) -> bool {
        let table = locked(&self.table);
        table.by_segment.contains_key(url) || table.by_root.contains_key(url)
    }

    fn lookup(&self, target: &str) -> Option<(Arc<Stream>, usize)> {
        locked(&self.table).by_segment.get(target).cloned()
    }

    pub fn nudge(&self, root: &str) {
        if let Some(stream) = locked(&self.table).by_root.get(root) {
            stream.nudge.notify_waiters();
        }
    }

    pub fn grow(&self, root: &str) {
        let Some(stream) = locked(&self.table).by_root.get(root).cloned() else { return };
        if self.cap(&stream) as u32 >= MAX_CHUNKS_AHEAD || self.ahead(&stream) != self.cap(&stream) {
            return;
        }
        stream.grown.fetch_add(1, Ordering::SeqCst);
        stream.nudge.notify_waiters();
        self.announce(&stream);
    }
}

impl ProxyShared {
    async fn playhead_position(&self, stream: &Stream) -> Option<u64> {
        let playhead = self.windows.playhead()?;
        tokio::time::timeout(self.config.probe_limit, playhead.position_ms(&stream.root)).await.ok().flatten()
    }

    pub(super) async fn hold_for_playhead(&self, target: &str, key: &str) {
        let Some((stream, index)) = self.windows.lookup(target) else { return };
        let deadline = Instant::now() + self.config.hold_ceiling;
        loop {
            let nudged = stream.nudge.notified();
            tokio::pin!(nudged);
            nudged.as_mut().enable();
            let ahead = self.windows.ahead(&stream);
            if index <= ahead || self.store.contains(key).await || locked(&self.flights).contains_key(key) {
                return;
            }
            let Some(position) = self.playhead_position(&stream).await else { return };
            if index <= stream.index_at(position) + ahead {
                return;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                crate::trace!("window", "hold ceiling reached for {}", crate::trace::tail(target));
                return;
            }
            let opens_at = stream.starts_ms[index - ahead];
            let wait = Duration::from_millis(opens_at.saturating_sub(position)).clamp(RECHECK_MIN, RECHECK_MAX);
            tokio::select! {
                _ = tokio::time::sleep(wait.min(left)) => {}
                _ = &mut nudged => {}
            }
        }
    }
}

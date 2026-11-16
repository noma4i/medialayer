use super::*;
use crate::api::test_support::{shared, FakeSettings};
use crate::api::{MediaItem, PlayerSlotFactory};
use crate::ffi::layer_harness::Offline;
use crate::ffi::{default_limits, default_pool_settings, DeviceMemory, OpenConfig};
use std::sync::Arc;


pub(super) fn config(dir: &std::path::Path, slots: Option<Arc<dyn PlayerSlotFactory>>, limits: Option<IngressLimits>) -> OpenConfig {
    OpenConfig {
        dir: dir.to_string_lossy().to_string(),
        fetcher: Arc::new(Offline),
        settings: shared::<FakeSettings>(),
        slots,
        device: DeviceMemory { total_mb: 4096, heap_class_mb: None, low_ram: false },
        defaults: default_limits(),
        pool: default_pool_settings(),
        namespace_headers: Vec::new(),
        image_windows: Vec::new(),
        forward_headers: None,
        evict_video: None,
        evict_image: None,
        evict_audio: None,
        memory_releaser: None,
        silent: false,
        ingress: limits,
        audio_focus: crate::audio::AudioFocus::Exclusive,
    }
}

pub(super) fn caller() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap()
}

pub(super) fn tight(entries: u32, owners: u32, bytes: u64) -> Option<IngressLimits> {
    Some(IngressLimits { entries, owners, bytes })
}

pub(super) fn update(owner: u64) -> Declared {
    Declared::PlaceUpdate { owner, input: PlaceInput { playing: true, ..PlaceInput::default() } }
}

pub(super) fn opened(queue: &Arc<InputQueue>, owner: u64) {
    queue.open(owner, OwnerKind::Place, Declared::PlaceOpen { owner, item: MediaItem::bare_video("https://o.test/a.mp4") }).unwrap();
}

pub(super) fn queued(queue: &Arc<InputQueue>) -> Vec<&'static str> {
    let mut order = Vec::new();
    loop {
        match queue.next() {
            queue::Upcoming::Apply(popped) => order.push(match popped.command {
                Declared::PlaceOpen { .. } => "open",
                Declared::PlaceUpdate { .. } => "update",
                Declared::PlaceSeek { .. } => "seek",
                Declared::KeySeek { .. } => "key-seek",
                Declared::Query { .. } => "query",
                Declared::Audio(CoreAudio::CallHold(true)) => "call-hold",
                Declared::Audio(CoreAudio::CallHold(false)) => "call-end",
                Declared::Audio(CoreAudio::PauseAll) => "pause-all",
                Declared::Audio(CoreAudio::ForgetPaused) => "forget-paused",
                Declared::Audio(CoreAudio::ResumePaused) => "resume-paused",
                _ => "other",
            }),
            queue::Upcoming::Release(..) => order.push("release"),
            _ => return order,
        }
    }
}

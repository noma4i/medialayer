use super::*;
use super::ingress::PlaceInput;
use crate::api::test_support::FakeSettings;
use crate::api::{MediaError, MediaItem, MediaState, MediaStats, PlayerSlotFactory};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub(super) struct Offline;

#[async_trait]
impl Fetcher for Offline {
    async fn fetch(&self, _request: super::FetchRequest) -> Result<super::FetchResponse, MediaError> {
        Err(MediaError::Fetch { detail: "offline".to_string() })
    }
}

pub(super) struct Served(pub(super) Mutex<Option<Vec<u8>>>);

#[async_trait]
impl BodyReader for Served {
    async fn next_chunk(&self) -> Result<Option<Vec<u8>>, MediaError> {
        Ok(self.0.lock().unwrap().take())
    }

    fn release(&self) {}
}

#[derive(Default)]
pub(super) struct Origin {
    pub(super) seen: Mutex<Vec<super::FetchRequest>>,
}

#[async_trait]
impl Fetcher for Origin {
    async fn fetch(&self, request: super::FetchRequest) -> Result<super::FetchResponse, MediaError> {
        self.seen.lock().unwrap().push(request);
        let body = Arc::new(Served(Mutex::new(Some(b"origin bytes".to_vec()))));
        Ok(super::FetchResponse { status: 200, headers: HashMap::from([("Content-Type".to_string(), "video/mp4".to_string())]), body })
    }
}

pub(super) struct Seen(pub(super) Mutex<Vec<MediaState>>);

impl StateListener for Seen {
    fn changed(&self, state: MediaState) {
        self.0.lock().unwrap().push(state);
    }

    fn failed(&self, _: MediaError) {}
}

pub(super) struct Totals(pub(super) Mutex<Vec<u64>>);

impl StatsListener for Totals {
    fn changed(&self, stats: MediaStats) {
        self.0.lock().unwrap().push(stats.total_bytes);
    }

    fn failed(&self, _: MediaError) {}
}

pub(super) fn config(dir: &std::path::Path, fetcher: Arc<dyn Fetcher>, settings: Arc<FakeSettings>, slots: Option<Arc<dyn PlayerSlotFactory>>) -> OpenConfig {
    OpenConfig {
        available_bytes: u64::MAX,
        dir: dir.to_string_lossy().to_string(),
        fetcher,
        settings,
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
        ingress: None,
    }
}

pub(super) fn ring(layer: &MediaLayer, active: u32, sources: Vec<Option<MediaItem>>) -> Arc<MediaWindow> {
    let window = layer.open_video_window().unwrap();
    window.replace(active, sources).unwrap();
    window
}

pub(super) fn place(layer: &MediaLayer, url: &str, input: PlaceInput) -> Arc<MediaPlace> {
    let place = layer.open_place(MediaItem::bare_video(url)).unwrap();
    place.update(input).unwrap();
    place
}

pub(super) fn playing() -> PlaceInput {
    PlaceInput { playing: true, ..PlaceInput::default() }
}

pub(super) fn caller() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap()
}

pub(super) const SLOW: std::time::Duration = std::time::Duration::from_secs(2);

pub(super) async fn pending_once(closed: impl std::future::Future<Output = ()>) -> bool {
    tokio::select! {
        biased;
        () = closed => false,
        () = std::future::ready(()) => true,
    }
}

use std::sync::{Arc, Mutex, PoisonError};

#[derive(Clone, Default, Debug, PartialEq, Eq, uniffi::Record)]
pub struct ImageSnapshot {
    pub requests: u64,
    pub decodes: u64,
    pub memory_hits: u64,
    pub disk_hits: u64,
    pub decoded_pixels: u64,
    pub placement_pixels: u64,
    pub bitmap_bytes: u64,
    pub active_requests: u64,
    pub active_observers: u64,
    pub cancellations: u64,
}

#[derive(uniffi::Enum)]
pub enum ImageWork {
    Request { placement_pixels: u64 },
    Decode { width: u32, height: u32, bytes: u64 },
    Finish { memory_hit: bool },
    Cancel { key: String },
    DiskHit,
    Observe,
    Unobserve,
}

#[derive(Default, uniffi::Object)]
pub struct ImageCounters {
    state: Mutex<ImageSnapshot>,
}

#[uniffi::export]
impl ImageCounters {
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> { Arc::new(Self::default()) }

    pub fn snapshot(&self) -> ImageSnapshot { self.state.lock().unwrap_or_else(PoisonError::into_inner).clone() }

    pub fn record(&self, work: ImageWork) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        match work {
            ImageWork::Request { placement_pixels } => {
                state.requests += 1;
                state.active_requests += 1;
                state.placement_pixels += placement_pixels;
            }
            ImageWork::Decode { width, height, bytes } => {
                state.decodes += 1;
                state.decoded_pixels += u64::from(width) * u64::from(height);
                state.bitmap_bytes += bytes;
            }
            ImageWork::Finish { memory_hit } => {
                state.active_requests -= 1;
                state.memory_hits += u64::from(memory_hit);
            }
            ImageWork::Cancel { .. } => {
                state.active_requests -= 1;
                state.cancellations += 1;
            }
            ImageWork::DiskHit => state.disk_hits += 1,
            ImageWork::Observe => state.active_observers += 1,
            ImageWork::Unobserve => state.active_observers -= 1,
        }
    }
}

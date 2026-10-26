use super::MAX_PIXELS;
use crate::memory::Reservation;
use std::sync::Mutex;

#[derive(uniffi::Object)]
pub struct DecodeHold {
    held: Mutex<Option<Reservation>>,
}

impl DecodeHold {
    pub(crate) fn new(reservation: Reservation) -> Self {
        Self { held: Mutex::new(Some(reservation)) }
    }
}

#[uniffi::export]
impl DecodeHold {
    pub fn bytes(&self) -> u64 {
        self.held.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).as_ref().map_or(0, Reservation::bytes)
    }

    pub fn release(&self) {
        self.held.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take();
    }
}

#[uniffi::export]
pub fn decode_cost(width: u32, height: u32) -> Option<u64> {
    let limit = MAX_PIXELS as u32;
    if width == 0 || height == 0 || width > limit || height > limit {
        return None;
    }
    Some(u64::from(width) * u64::from(height) * 4)
}

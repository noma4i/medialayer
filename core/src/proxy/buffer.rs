use super::config::Config;
use crate::memory::{Ledger, Owner, Refused, Reservation};
use bytes::Bytes;
use std::time::Duration;

#[derive(Clone)]
pub(super) struct Buffers {
    ledger: Ledger,
    chunk: usize,
    floor: usize,
    wait: Duration,
}

struct Held {
    data: Bytes,
    _reservation: Reservation,
}

impl AsRef<[u8]> for Held {
    fn as_ref(&self) -> &[u8] {
        &self.data
    }
}

impl Buffers {
    pub fn new(config: &Config, wait: Duration) -> Buffers {
        Buffers { ledger: config.ledger.clone(), chunk: config.stream_chunk, floor: config.min_buffer, wait }
    }

    pub fn floor(&self) -> usize {
        self.floor
    }

    pub async fn reserve_chunk(&self) -> Result<Reservation, Refused> {
        self.reserve_between(self.floor, self.chunk).await
    }

    pub async fn reserve_exact(&self, bytes: usize) -> Result<Reservation, Refused> {
        self.reserve_between(bytes, bytes).await
    }

    pub async fn reserve_between(&self, floor: usize, wanted: usize) -> Result<Reservation, Refused> {
        self.reserve_for(Owner::ProxyBuffer, floor, wanted).await
    }

    pub async fn reserve_decode(&self, bytes: usize) -> Result<Reservation, Refused> {
        self.reserve_for(Owner::ImageDecode, bytes, bytes).await
    }

    async fn reserve_for(&self, owner: Owner, floor: usize, wanted: usize) -> Result<Reservation, Refused> {
        let reserved = self.ledger.reserve_between(owner, floor as u64, wanted as u64);
        tokio::time::timeout(self.wait, reserved).await.unwrap_or(Err(Refused::Ceiling { owner, requested: floor as u64, available: 0 }))
    }

    pub fn hold(data: Bytes, reservation: Reservation) -> Bytes {
        Bytes::from_owner(Held { data, _reservation: reservation })
    }
}

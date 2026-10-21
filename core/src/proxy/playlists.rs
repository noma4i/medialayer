use super::inner::ProxyShared;
use crate::hls::Playlist;
use crate::memory::{Owner, Reservation};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

const MAX_PLANS: usize = 256;

pub struct StoredPlaylist {
    pub plan: Playlist,
    _held: Reservation,
}

struct PlaylistBody(Arc<StoredPlaylist>);

impl AsRef<[u8]> for PlaylistBody {
    fn as_ref(&self) -> &[u8] { &self.0.plan.body }
}

impl StoredPlaylist {
    pub fn body(self: &Arc<Self>) -> bytes::Bytes {
        bytes::Bytes::from_owner(PlaylistBody(self.clone()))
    }


}

#[derive(Default)]
struct Entries {
    epoch: u64,
    plans: HashMap<String, (u64, Arc<StoredPlaylist>)>,
}

#[derive(Default)]
pub(super) struct Playlists(Mutex<Entries>);

impl Playlists {
    pub fn forget(&self, url: Option<&str>) {
        let mut entries = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        entries.epoch += 1;
        match url {
            Some(url) => entries.plans.retain(|key, _| key != url && !key.strip_prefix(url).is_some_and(|suffix| suffix.starts_with("#ns:"))),
            None => entries.plans = HashMap::new(),
        }
    }
}

impl ProxyShared {
    pub async fn stored_plan(&self, target: &str, key: &str) -> Result<Option<Arc<StoredPlaylist>>, &'static str> {
        let lock = self.probe_lock(key);
        let _turn = lock.lock.lock().await;
        let epoch = self.playlists.0.lock().unwrap_or_else(PoisonError::into_inner).epoch;
        let Some(mut reader) = self.store.open_reader(key).await else {
            self.playlists.forget(Some(key));
            return Ok(None);
        };
        let Some((version, length)) = reader.version() else { return Ok(None) };
        {
            let mut entries = self.playlists.0.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some((stored_version, plan)) = entries.plans.get(key) {
                if *stored_version == version && (plan.plan.is_static || self.playlist_is_fresh(key)) {
                    return Ok(Some(plan.clone()));
                }
                entries.plans.remove(key);
            }
        }
        if length > self.config.max_playlist_bytes { return Err("playlist too large"); }
        if self.config.ledger.used().saturating_add(length) > self.config.ledger.ceiling() {
            self.playlists.forget(None);
        }
        let held = self.buffers().reserve_exact(length as usize).await.map_err(|_| "no memory for the playlist")?;
        let bytes = reader.get_range(0, length).await.ok_or("playlist vanished from the store")?;
        let plan = Playlist::parse(&String::from_utf8_lossy(&bytes), target);
        drop(bytes);
        drop(held);
        if !plan.is_static && !self.playlist_is_fresh(key) { return Ok(None); }
        let memory = plan.memory_bytes().saturating_add(key.len() + 128) as u64;
        let mut entries = self.playlists.0.lock().unwrap_or_else(PoisonError::into_inner);
        if entries.plans.len() >= MAX_PLANS { entries.plans.clear(); }
        let reservation = self.config.ledger.reserve(Owner::ProxyBuffer, memory).or_else(|_| {
            entries.plans.clear();
            self.config.ledger.reserve(Owner::ProxyBuffer, memory)
        }).map_err(|_| "no memory for the playlist plan")?;
        let plan = Arc::new(StoredPlaylist { plan, _held: reservation });
        if entries.epoch == epoch { entries.plans.insert(key.to_string(), (version, plan.clone())); }
        Ok(Some(plan))
    }
}

#[cfg(test)]
#[path = "playlists_tests.rs"]
mod tests;

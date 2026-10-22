use super::inner::ProxyShared;
use super::recent::Recent;
use crate::hls::Playlist;
use crate::memory::{Owner, Reservation};
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

struct Entries {
    epoch: u64,
    plans: Recent<(u64, Arc<StoredPlaylist>)>,
}

pub(super) struct Playlists(Mutex<Entries>);

impl Default for Playlists {
    fn default() -> Self {
        Playlists(Mutex::new(Entries { epoch: 0, plans: Recent::new(MAX_PLANS) }))
    }
}

impl Playlists {
    pub fn forget(&self, url: Option<&str>) {
        let mut entries = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        entries.epoch += 1;
        match url {
            Some(url) => entries.plans.retain(|key, _| key != url && !key.strip_prefix(url).is_some_and(|suffix| suffix.starts_with("#ns:"))),
            None => entries.plans.clear(),
        }
    }
}

#[cfg(test)]
impl Playlists {
    pub fn len(&self) -> usize {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).plans.len()
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
            if let Some((stored_version, plan)) = entries.plans.get(key).cloned() {
                if stored_version == version && (plan.plan.is_static || self.playlist_is_fresh(key)) {
                    return Ok(Some(plan));
                }
                entries.plans.remove(key);
            }
        }
        if length > self.config.max_playlist_bytes { return Err("playlist too large"); }
        while self.config.ledger.used().saturating_add(length) > self.config.ledger.ceiling() {
            if self.playlists.0.lock().unwrap_or_else(PoisonError::into_inner).plans.pop_oldest().is_none() {
                break;
            }
        }
        let held = self.buffers().reserve_exact(length as usize).await.map_err(|_| "no memory for the playlist")?;
        let Some(bytes) = reader.get_range(0, length).await else { return Ok(None) };
        let plan = Playlist::parse(&String::from_utf8_lossy(&bytes), target);
        drop(bytes);
        drop(held);
        if !plan.is_static && !self.playlist_is_fresh(key) { return Ok(None); }
        let memory = plan.memory_bytes().saturating_add(key.len() + 128) as u64;
        let mut entries = self.playlists.0.lock().unwrap_or_else(PoisonError::into_inner);
        let reservation = loop {
            match self.config.ledger.reserve(Owner::ProxyBuffer, memory) {
                Ok(reservation) => break reservation,
                Err(_) if entries.plans.pop_oldest().is_some() => {}
                Err(_) => return Err("no memory for the playlist plan"),
            }
        };
        let plan = Arc::new(StoredPlaylist { plan, _held: reservation });
        if entries.epoch == epoch { entries.plans.insert(key, (version, plan.clone()), |_, _| false); }
        Ok(Some(plan))
    }
}

#[cfg(test)]
#[path = "playlists_tests.rs"]
mod tests;

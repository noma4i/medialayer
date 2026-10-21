use super::*;
use crate::proxy::{Config, memory_store::MemoryStore, tasks::Tasks};

#[tokio::test]
async fn inv_proxy_13_playlist_plans_obey_entry_and_memory_limits() {
    let store = MemoryStore::new();
    let config = Config::default();
    let ledger = config.ledger.clone();
    let inner = ProxyShared::new(config, Arc::new(crate::fetch::mock::MockFetcher::new()), Arc::new(store.clone()), Tasks::default());
    for index in 0..=MAX_PLANS {
        let url = format!("https://limit.test/{index}.m3u8");
        store.seed(&url, b"#EXTM3U\n#EXTINF:2,\na.ts\n#EXT-X-ENDLIST\n");
        drop(inner.stored_plan(&url, &url).await.unwrap().unwrap());
    }
    assert!(inner.playlists.0.lock().unwrap().plans.len() <= MAX_PLANS);
    inner.playlists.forget(None);
    assert_eq!(ledger.used(), 0);
    ledger.set_ceiling(128);
    assert!(inner.stored_plan("https://limit.test/0.m3u8", "https://limit.test/0.m3u8").await.is_err());
    assert_eq!(ledger.used(), 0);
}

impl StoredPlaylist {
    pub(crate) fn for_test(text: &str, url: &str) -> Arc<Self> {
        let plan = Playlist::parse(text, url);
        let held = crate::memory::Ledger::unbounded().reserve(Owner::ProxyBuffer, plan.memory_bytes() as u64).unwrap();
        Arc::new(Self { plan, _held: held })
    }
}

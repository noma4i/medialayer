use super::memory_store::MemoryStore;
use super::*;
use crate::fetch::mock::MockFetcher;
use crate::hls::Playlist;

const MEDIA: &str = "https://o.test/hls/media.m3u8";
const VOD: &str = "#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXTINF:2.0,\nseg0.ts\n#EXTINF:2.0,\nseg1.ts\n#EXT-X-ENDLIST\n";

async fn proxy_with(store: MemoryStore) -> Proxy {
    Proxy::start(Config::default(), Arc::new(MockFetcher::new()), Arc::new(store)).await.unwrap()
}

fn authorization() -> HashMap<String, String> {
    HashMap::from([("Authorization".to_string(), "user-a".to_string())])
}

fn carries_authorization(proxy: &Proxy, url: &str) -> bool {
    proxy.inner.upstream_headers(url, &HashMap::new()).contains_key("Authorization")
}

fn other(number: usize) -> String {
    format!("https://o.test/other{number}.mp4")
}

#[tokio::test]
async fn inv_proxy_32_a_route_that_keeps_being_requested_keeps_its_headers_after_4097_registrations() {
    let proxy = proxy_with(MemoryStore::new()).await;
    let live = "https://o.test/live.mp4";
    proxy.inner.register_headers(live, &authorization());
    for number in 0..4097 {
        proxy.inner.register_headers(&other(number), &HashMap::new());
        if number % 1000 == 0 {
            assert!(carries_authorization(&proxy, live), "lost after {number} registrations");
        }
    }

    assert!(carries_authorization(&proxy, live));
}

#[tokio::test]
async fn inv_proxy_32_segments_of_a_registered_plan_keep_the_inherited_headers_under_a_flood_of_registrations() {
    let proxy = proxy_with(MemoryStore::new()).await;
    let plan = Playlist::parse(VOD, MEDIA);
    proxy.inner.inherit_headers(&plan.references, &authorization());
    proxy.inner.windows.register_plan(MEDIA, &plan);
    for number in 0..5000 {
        proxy.inner.register_headers(&other(number), &HashMap::new());
    }

    for segment in &plan.references {
        assert!(carries_authorization(&proxy, segment), "{segment}");
    }
}

#[tokio::test]
async fn inv_proxy_33_totals_and_range_flags_evict_the_oldest_entry_and_keep_the_rest() {
    let proxy = proxy_with(MemoryStore::new()).await;
    for number in 0..1025 {
        proxy.inner.remember_total(&other(number), 7);
        proxy.inner.mark_ignores_range(&other(number));
    }

    assert_eq!(proxy.inner.known_total(&other(0)), None);
    assert!(!proxy.inner.ignores_range(&other(0)));
    for number in 1..1025 {
        assert_eq!(proxy.inner.known_total(&other(number)), Some(7), "total {number}");
        assert!(proxy.inner.ignores_range(&other(number)), "flag {number}");
    }
}

#[tokio::test]
async fn inv_proxy_33_playlist_stamps_stay_within_their_limit_and_keep_the_newest() {
    let proxy = proxy_with(MemoryStore::new()).await;
    for number in 0..300 {
        proxy.inner.stamp_playlist(&other(number));
    }

    assert_eq!(proxy.inner.playlist_stamps.lock().unwrap().len(), 256);
    assert!(proxy.inner.playlist_is_fresh(&other(299)));
    assert!(proxy.inner.playlist_is_fresh(&other(44)));
    assert!(!proxy.inner.playlist_is_fresh(&other(43)));
}

#[tokio::test]
async fn inv_proxy_33_stored_plans_evict_the_oldest_plan_and_keep_the_rest() {
    let store = MemoryStore::new();
    let proxy = proxy_with(store.clone()).await;
    for number in 0..257 {
        let url = format!("https://o.test/hls/{number}.m3u8");
        store.seed(&url, VOD.as_bytes());
        assert!(proxy.inner.stored_plan(&url, &url).await.unwrap().is_some());
    }

    assert_eq!(proxy.inner.playlists.len(), 256);
}

#[tokio::test]
async fn inv_proxy_33_variant_links_evict_the_oldest_link_and_keep_the_rest() {
    let proxy = proxy_with(MemoryStore::new()).await;
    for number in 0..33 {
        proxy.inner.windows.remember_variant(&format!("https://o.test/v{number}.m3u8"), &format!("https://o.test/root{number}.m3u8"));
    }

    assert_eq!(proxy.inner.windows.variant_count(), 32);
    assert_eq!(proxy.inner.windows.variant_root("https://o.test/v0.m3u8"), None);
    assert_eq!(proxy.inner.windows.variant_root("https://o.test/v32.m3u8").as_deref(), Some("https://o.test/root32.m3u8"));
}

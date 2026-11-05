use super::*;
use std::collections::HashMap;
use crate::proxy::memory_store::MemoryStore;
use crate::proxy::Proxy;

const TARGET: &str = "https://cdn.test/p.m3u8";
const FIRST: &[u8] = b"#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXTINF:2,\na.ts\n#EXTINF:2,\nb.ts\n#EXT-X-ENDLIST\n";
const SECOND: &[u8] = b"#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXTINF:2,\nc.ts\n#EXTINF:2,\nd.ts\n#EXT-X-ENDLIST\n";

fn authorization(value: &str) -> HashMap<String, String> {
    HashMap::from([("authorization".to_string(), value.to_string())])
}

async fn answer(inner: &Arc<ProxyShared>, headers: &HashMap<String, String>) {
    assert_eq!(playlist(inner, TARGET, headers.clone(), false).await.status(), 200);
}

#[tokio::test]
async fn inv_proxy_42_a_repeated_stored_playlist_with_the_same_headers_registers_nothing_again() {
    let store = MemoryStore::new();
    store.seed(TARGET, FIRST);
    let proxy = Proxy::start(Default::default(), Arc::new(crate::fetch::mock::MockFetcher::new()), Arc::new(store.clone())).await.unwrap();
    let inner = proxy.inner.clone();
    let original = authorization("one");
    answer(&inner, &original).await;
    let segment = proxy.playlist_plan(TARGET, &original).await.unwrap().plan.references[0].clone();
    let stream = inner.windows.stream_id(TARGET).unwrap();
    assert!(inner.registered_headers_of(&segment).is_some());

    inner.unregister_headers(&segment);
    answer(&inner, &original).await;
    assert_eq!(inner.windows.stream_id(TARGET), Some(stream), "the window was registered again");
    assert!(inner.registered_headers_of(&segment).is_none(), "the references were visited again");

    answer(&inner, &authorization("two")).await;
    assert_ne!(inner.windows.stream_id(TARGET), Some(stream), "changed headers did not register the window");
    assert_eq!(inner.registered_headers_of(&segment), Some(authorization("two")));

    let stream = inner.windows.stream_id(TARGET).unwrap();
    store.seed(TARGET, SECOND);
    proxy.forget_playlists(Some(TARGET));
    answer(&inner, &authorization("two")).await;
    assert_ne!(inner.windows.stream_id(TARGET), Some(stream), "another plan did not register the window");
    let replaced = proxy.playlist_plan(TARGET, &authorization("two")).await.unwrap().plan.references[0].clone();
    assert!(inner.registered_headers_of(&replaced).is_some());
    proxy.stop().await;
}

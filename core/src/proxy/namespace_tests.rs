use super::memory_store::MemoryStore;
use super::test_client::get;
use super::*;
use crate::fetch::mock::{MockFetcher, Script};
use url::Url;

const URL: &str = "https://o.test/clip.mp4";

async fn rig(namespace: &[&str]) -> (Proxy, Arc<MockFetcher>, MemoryStore) {
    let config = Config { namespace_headers: namespace.iter().map(|name| name.to_string()).collect(), ..Config::default() };
    let fetcher = Arc::new(MockFetcher::new());
    fetcher.route(URL, Script::ok(vec![7; 3000]));
    let store = MemoryStore::new();
    let proxy = Proxy::start(config, fetcher.clone(), Arc::new(store.clone())).await.unwrap();
    (proxy, fetcher, store)
}

fn authorization(value: &str) -> HashMap<String, String> {
    HashMap::from([("Authorization".to_string(), value.to_string())])
}

async fn play_as(proxy: &Proxy, token: &str) -> u16 {
    let routed = proxy.url_for(URL, &authorization(token)).unwrap();
    let parsed = Url::parse(&routed).unwrap();
    get(proxy.port(), &format!("{}?{}", parsed.path(), parsed.query().unwrap())).await.status
}

#[tokio::test]
async fn inv_proxy_15_one_url_with_different_authorization_is_cached_per_user_when_the_option_lists_it() {
    let (proxy, fetcher, store) = rig(&["Authorization"]).await;

    assert_eq!(play_as(&proxy, "user-a").await, 200);
    assert_eq!(play_as(&proxy, "user-b").await, 200);
    assert_eq!(play_as(&proxy, "user-a").await, 200);

    assert_eq!(fetcher.calls_for(URL), 2);
    assert_eq!(store.count(), 2);
}

#[tokio::test]
async fn inv_proxy_15_by_default_the_url_alone_is_the_key() {
    let (proxy, fetcher, store) = rig(&[]).await;

    play_as(&proxy, "user-a").await;
    play_as(&proxy, "user-b").await;

    assert_eq!(fetcher.calls_for(URL), 1);
    assert_eq!(store.count(), 1);
    assert!(store.bytes(URL).is_some());
}

#[tokio::test]
async fn inv_proxy_15_prefetch_and_the_ready_check_use_the_same_namespaced_key() {
    let (proxy, _fetcher, store) = rig(&["Authorization"]).await;

    proxy.fetch_into_store(URL, &authorization("user-a"), Priority::Prefetch, None).await.unwrap();

    let key = proxy.key_for(URL, &authorization("user-a"));
    assert!(store.bytes(&key).is_some());
    assert_ne!(key, proxy.key_for(URL, &authorization("user-b")));
    assert_eq!(proxy.key_for(URL, &HashMap::new()), URL);
}

#[tokio::test]
async fn inv_proxy_15_empty_declarations_replace_authorization_and_cookie() {
    for name in ["Authorization", "Cookie"] {
        let (proxy, fetcher, store) = rig(&[name]).await;
        for headers in [HashMap::from([(name.to_string(), "user-a".to_string())]), HashMap::new()] {
            let routed = proxy.url_for(URL, &headers).unwrap();
            let parsed = Url::parse(&routed).unwrap();
            assert_eq!(get(proxy.port(), &format!("{}?{}", parsed.path(), parsed.query().unwrap())).await.status, 200);
        }
        assert_eq!(fetcher.calls_for(URL), 2, "anonymous declaration reused {name}");
        assert_eq!(store.count(), 2);
        assert!(!fetcher.requests().last().unwrap().headers.keys().any(|header| header.eq_ignore_ascii_case(name)));
    }
}

#[tokio::test]
async fn inv_proxy_15_hls_inheritance_replaces_old_headers_but_preserves_explicit_children() {
    for name in ["Authorization", "Cookie"] {
        let (proxy, _, _) = rig(&[name]).await;
        let child = "https://o.test/segment.ts".to_string();
        let explicit = "https://o.test/explicit.ts".to_string();
        proxy.inner.register_headers(&explicit, &HashMap::new());
        for headers in [
            HashMap::from([(name.to_string(), "user-a".to_string())]),
            HashMap::new(),
            HashMap::from([(name.to_string(), "user-b".to_string())]),
        ] {
            proxy.inner.inherit_headers(&[child.clone(), explicit.clone()], &headers);
            let identity = HashMap::from([("Accept-Encoding".to_string(), "identity".to_string())]);
            let mut expected = headers;
            expected.extend(identity.clone());
            assert_eq!(proxy.inner.upstream_headers(&child, &HashMap::new()), expected);
            assert_eq!(proxy.inner.upstream_headers(&explicit, &HashMap::new()), identity);
        }
    }
}

#[tokio::test]
async fn inv_proxy_29_client_gzip_requests_identity_and_receives_playlist_text() {
    let url = "https://o.test/live.m3u8";
    let playlist = "#EXTM3U\n#EXT-X-TARGETDURATION:6\n#EXT-X-ENDLIST\n";
    let fetcher = Arc::new(MockFetcher::new());
    fetcher.route(url, Script::ok(playlist));
    let proxy = Proxy::start(Config::default(), fetcher.clone(), Arc::new(MemoryStore::new())).await.unwrap();
    let routed = Url::parse(&proxy.url_for(url, &HashMap::new()).unwrap()).unwrap();
    let reply = super::test_client::request(
        proxy.port(), "GET", &format!("{}?{}", routed.path(), routed.query().unwrap()),
        &[("Accept-Encoding", "gzip")],
    ).await;
    let requests = fetcher.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(crate::fetch::header_value(&requests[0].headers, "accept-encoding"), Some("identity"));
    assert_eq!(reply.status, 200);
    assert_eq!(String::from_utf8(reply.body).unwrap(), playlist);
}

#[tokio::test]
async fn inv_proxy_29_custom_forward_headers_cannot_override_identity() {
    let config = Config {
        policy: crate::fetch::FetchPolicy::default().with_forwarded_headers(&["Accept-Encoding"]),
        ..Config::default()
    };
    let fetcher = Arc::new(MockFetcher::new());
    fetcher.route(URL, Script::ok(vec![7; 3000]));
    let proxy = Proxy::start(config, fetcher.clone(), Arc::new(MemoryStore::new())).await.unwrap();
    let headers = HashMap::from([
        ("aCcEpT-EnCoDiNg".to_string(), "gzip".to_string()),
        ("ACCEPT-ENCODING".to_string(), "br".to_string()),
    ]);
    proxy.fetch_into_store(URL, &headers, Priority::Prefetch, None).await.unwrap();
    let requests = fetcher.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].headers, HashMap::from([("Accept-Encoding".to_string(), "identity".to_string())]));
}

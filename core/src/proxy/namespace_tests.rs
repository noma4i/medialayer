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
async fn one_url_with_different_authorization_is_cached_per_user_when_the_option_lists_it() {
    let (proxy, fetcher, store) = rig(&["Authorization"]).await;

    assert_eq!(play_as(&proxy, "user-a").await, 200);
    assert_eq!(play_as(&proxy, "user-b").await, 200);
    assert_eq!(play_as(&proxy, "user-a").await, 200);

    assert_eq!(fetcher.calls_for(URL), 2);
    assert_eq!(store.count(), 2);
}

#[tokio::test]
async fn by_default_the_url_alone_is_the_key() {
    let (proxy, fetcher, store) = rig(&[]).await;

    play_as(&proxy, "user-a").await;
    play_as(&proxy, "user-b").await;

    assert_eq!(fetcher.calls_for(URL), 1);
    assert_eq!(store.count(), 1);
    assert!(store.bytes(URL).is_some());
}

#[tokio::test]
async fn prefetch_and_the_ready_check_use_the_same_namespaced_key() {
    let (proxy, _fetcher, store) = rig(&["Authorization"]).await;

    proxy.fetch_into_store(URL, &authorization("user-a"), Priority::Prefetch).await.unwrap();

    let key = proxy.key_for(URL, &authorization("user-a"));
    assert!(store.bytes(&key).is_some());
    assert_ne!(key, proxy.key_for(URL, &authorization("user-b")));
    assert_eq!(proxy.key_for(URL, &HashMap::new()), URL);
}

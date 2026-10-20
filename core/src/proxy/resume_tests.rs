use super::test_client::open;
use super::*;
use crate::cache::{classify_kind, Cache, CacheStore};
use crate::fetch::mock::{MockFetcher, Script};
use crate::limits::Limits;
use tokio::io::AsyncReadExt;
use url::Url;

const URL: &str = "https://o.test/long.mp4";
const SIZE: usize = 4 * 1024 * 1024;

fn pattern() -> Vec<u8> {
    (0..SIZE).map(|index| (index % 251) as u8).collect()
}

struct Rig {
    proxy: Proxy,
    cache: Cache,
    fetcher: Arc<MockFetcher>,
    _dir: tempfile::TempDir,
}

async fn rig() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let cache = Cache::open(dir.path(), Limits { budget: 8 * 1024 * 1024, min_budget: 1, ..Limits::default() });
    cache.ready().await;
    let fetcher = Arc::new(MockFetcher::new());
    fetcher.route(URL, Script::ok(pattern()));
    let store = Arc::new(CacheStore::new(cache.clone(), Arc::new(classify_kind)));
    let proxy = Proxy::start(Config::default(), fetcher.clone(), store).await.unwrap();
    proxy.fetch_into_store(URL, &HashMap::new(), Priority::Prefetch, None).await.unwrap();
    Rig { proxy, cache, fetcher, _dir: dir }
}

struct Client {
    stream: tokio::net::TcpStream,
    raw: Vec<u8>,
}

async fn client_paused_after_the_first_bytes(rig: &Rig) -> Client {
    let routed = rig.proxy.url_for(URL, &HashMap::new()).unwrap();
    let parsed = Url::parse(&routed).unwrap();
    let mut stream = open(rig.proxy.port(), "GET", &format!("{}?{}", parsed.path(), parsed.query().unwrap()), &[]).await;
    let mut raw = vec![0u8; 8192];
    let read = stream.read(&mut raw).await.unwrap();
    raw.truncate(read);
    Client { stream, raw }
}

async fn body_after_the_rest(mut client: Client) -> Vec<u8> {
    client.stream.read_to_end(&mut client.raw).await.unwrap();
    let split = client.raw.windows(4).position(|window| window == b"\r\n\r\n").unwrap();
    client.raw[split + 4..].to_vec()
}

#[tokio::test]
async fn inv_proxy_04_an_entry_removed_in_the_middle_of_a_response_is_continued_from_the_origin_with_the_right_bytes() {
    let rig = rig().await;
    let stream = client_paused_after_the_first_bytes(&rig).await;
    let calls_before = rig.fetcher.calls();
    assert!(rig.cache.remove(URL).await);

    let body = body_after_the_rest(stream).await;

    assert_eq!(body, pattern());
    assert!(rig.fetcher.calls() > calls_before, "the response never went back to the origin");
}

#[tokio::test]
async fn inv_proxy_04_an_entry_evicted_by_another_download_in_the_middle_of_a_response_is_continued_with_the_right_bytes() {
    let rig = rig().await;
    let stream = client_paused_after_the_first_bytes(&rig).await;
    rig.cache.set_limits(Limits { budget: 1024, min_budget: 1, ..Limits::default() }).await.unwrap();
    for _ in 0..100_000 {
        rig.cache.evict_now().await;
        if !rig.cache.contains(URL).await {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert!(!rig.cache.contains(URL).await, "the entry was never evicted");

    let body = body_after_the_rest(stream).await;

    assert_eq!(body, pattern());
}

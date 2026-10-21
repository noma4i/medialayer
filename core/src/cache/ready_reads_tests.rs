use super::concurrency_tests::write_prepared_dir;
use super::test_support::limits;
use super::*;
use crate::api::CacheControl;
use crate::fetch::mock::MockFetcher;
use crate::limits::EvictionStrategy;
use crate::proxy::{ByteStore, Config, Proxy};
use std::collections::HashMap;
use std::sync::Arc;

const KEY: &str = "https://o.test/media/abc/200.gif";

fn reopened_over_existing_files(dir: &std::path::Path) -> Cache {
    write_prepared_dir(dir, 3, KEY);
    let cache = Cache::open(dir, limits(u64::MAX / 2, 0.9, EvictionStrategy::Lru), u64::MAX);
    assert!(!cache.is_ready());
    cache
}

#[tokio::test(flavor = "current_thread")]
async fn inv_cache_19_the_store_adapter_sees_the_files_of_a_reopened_directory_before_the_index_is_ready() {
    let dir = tempfile::tempdir().unwrap();
    let cache = reopened_over_existing_files(dir.path());
    let store = CacheStore::new(cache.clone(), Arc::new(classify_kind));

    assert!(store.contains(KEY).await);
    assert_eq!(store.len(KEY).await, Some(10));
    assert!(store.meta(KEY).await.is_some());
    assert_eq!(store.get_range(KEY, 0, 10).await, Some(b"0123456789".to_vec()));
}

#[tokio::test(flavor = "current_thread")]
async fn inv_cache_19_the_cache_control_port_reports_cached_files_of_a_reopened_directory_before_the_index_is_ready() {
    let dir = tempfile::tempdir().unwrap();
    let cache = reopened_over_existing_files(dir.path());

    assert!(CacheControl::contains(&cache, KEY).await);
}

#[tokio::test(flavor = "current_thread")]
async fn inv_cache_19_an_image_fetch_right_after_reopening_reads_the_disk_and_never_asks_the_origin() {
    let dir = tempfile::tempdir().unwrap();
    let cache = reopened_over_existing_files(dir.path());
    let fetcher = Arc::new(MockFetcher::new());
    fetcher.set_offline(true);
    let store = CacheStore::new(cache, Arc::new(classify_kind));
    let proxy = Proxy::start(Config::default(), fetcher.clone(), Arc::new(store)).await.expect("proxy start");

    let bytes = proxy.fetch_bytes(KEY, &HashMap::new(), 1 << 20).await;

    assert_eq!(bytes.ok(), Some(b"0123456789".to_vec()));
    assert_eq!(fetcher.calls(), 0);
}

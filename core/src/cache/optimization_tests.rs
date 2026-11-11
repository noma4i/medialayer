use super::entry::{Check, Entry};
use super::state::State;
use super::test_support::*;
use super::*;
use crate::limits::{EvictionStrategy, Limits};
use crate::proxy::Meta;

fn entry() -> Entry {
    Entry { size: 1, check: Check(vec![0].into()), kind: Kind::Stream, created: 0, last_access: 0, hits: 0, meta: Meta::default(), tone: None, image_size: None }
}

#[tokio::test]
async fn inv_cache_01_replaced_files_remain_charged_until_the_last_reader_drops() {
    let f = fixture(limits(10, 0.9, EvictionStrategy::Lru), &[]).await;
    f.cache.put("clip", Kind::Image, b"old").await.unwrap();
    let old = f.cache.open_reader("clip").await.unwrap();
    f.cache.put("clip", Kind::Image, b"new").await.unwrap();
    let current = f.cache.open_reader("clip").await.unwrap();
    let mut writer = f.cache.begin_write("other", Kind::Image).await.unwrap();
    assert_eq!(writer.write(&[0; 5]).await, Err(CacheError::NotEnoughSpace));
    assert_eq!(f.cache.inner.state().reserved, 0);
    drop(old);
    writer.write(&[0; 5]).await.unwrap();
    assert_eq!(f.cache.inner.state().reserved, 5);
    assert_eq!(current.get_range(0, 3).await.as_deref(), Some(b"new".as_slice()));
}

#[tokio::test]
async fn inv_cache_07_cancelled_flush_preserves_dirty_state() {
    use std::sync::atomic::Ordering;
    let f = fixture(Limits::default(), &[]).await;
    let (_sender, receiver) = tokio::sync::oneshot::channel();
    let (entered, ready) = tokio::sync::oneshot::channel();
    *f.cache.inner.save_pause.lock().unwrap() = Some((entered, receiver));
    f.cache.inner.dirty.store(true, Ordering::SeqCst);
    let cache = f.cache.clone();
    let task = tokio::spawn(async move { cache.flush().await });
    ready.await.unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(f.cache.inner.dirty.load(Ordering::SeqCst));
}

#[tokio::test]
async fn inv_cache_07_last_owner_drop_saves_changes_after_the_snapshot() {
    let f = fixture(Limits::default(), &[]).await;
    let mut keys = ["first", "second"];
    keys.sort_by_key(|key| super::naming::file_name(key));
    let (resume, receiver) = tokio::sync::oneshot::channel();
    let (entered, ready) = tokio::sync::oneshot::channel();
    *f.cache.inner.save_pause.lock().unwrap() = Some((entered, receiver));
    f.cache.put(keys[1], Kind::Image, b"a").await.unwrap();
    ready.await.unwrap();
    f.cache.put(keys[0], Kind::Image, b"b").await.unwrap();
    let owner = std::sync::Arc::downgrade(&f.cache.inner);
    drop(f.cache);
    resume.send(()).unwrap();
    wait_until(|| owner.upgrade().is_none()).await;
    let reopened = Cache::open(f.dir.path(), Limits::default());
    reopened.ready().await;
    assert_eq!(reopened.get(keys[1]).await.as_deref(), Some(b"a".as_slice()));
    assert_eq!(reopened.get(keys[0]).await.as_deref(), Some(b"b".as_slice()));
}

#[tokio::test]
async fn inv_cache_03_old_reader_drop_does_not_unpin_a_replacement() {
    let f = fixture(Limits::default(), &[]).await;
    f.cache.put("clip", Kind::Image, b"old").await.unwrap();
    let old = f.cache.open_reader("clip").await.unwrap();
    f.cache.put("clip", Kind::Image, b"new").await.unwrap();
    let new = f.cache.open_reader("clip").await.unwrap();
    assert_eq!(old.get_range(0, 3).await.as_deref(), Some(b"old".as_slice()));
    assert_eq!(new.get_range(0, 3).await.as_deref(), Some(b"new".as_slice()));
    drop(old);
    let name = super::naming::file_name("clip");
    assert_eq!(f.cache.inner.state().slot(&name).unwrap().readers, 1);
    drop(new);
    assert_eq!(f.cache.inner.state().slot(&name).unwrap().readers, 0);
}

#[tokio::test]
async fn inv_cache_10_budget_is_the_configured_value_and_live_limit_changes_apply() {
    let dir = tempfile::tempdir().unwrap();
    let cache = Cache::open(dir.path(), Limits::default());
    cache.ready().await;
    assert_eq!(cache.inner.state().limits.budget, Limits::default().budget);
    let configured = Limits { budget: 1000, min_budget: 1, ..Limits::default() };
    cache.set_limits(configured).await.unwrap();
    let mut writer = cache.begin_write("clip", Kind::Stream).await.unwrap();
    writer.write(&[0; 1000]).await.unwrap();
    assert_eq!(writer.write(&[0]).await, Err(CacheError::NotEnoughSpace));
    drop(writer);
    cache.set_limits(Limits { budget: 500, ..configured }).await.unwrap();
    let mut writer = cache.begin_write("clip", Kind::Stream).await.unwrap();
    writer.write(&[0; 500]).await.unwrap();
    assert_eq!(writer.write(&[0]).await, Err(CacheError::NotEnoughSpace));
}

#[tokio::test]
async fn inv_cache_17_cold_read_reuses_the_file_opened_for_verification() {
    use std::sync::atomic::Ordering;
    let f = fixture(Limits::default(), &[]).await;
    f.cache.put("clip", Kind::Image, b"data").await.unwrap();
    f.cache.flush().await.unwrap();
    let cache = Cache::open(f.dir.path(), Limits::default());
    cache.ready().await;
    for _ in 0..2 {
        let reader = cache.open_reader("clip").await.unwrap();
        assert_eq!(reader.get_range(1, 2).await.as_deref(), Some(b"at".as_slice()));
    }
    assert_eq!(cache.inner.hashed_bytes.load(Ordering::Relaxed), 4);
    assert_eq!(cache.inner.read_opens.load(Ordering::Relaxed), 2);
    eprintln!("cold_then_warm opens={} hash_bytes={} read_jobs={} read_allocations={}", cache.inner.read_opens.load(Ordering::Relaxed), cache.inner.hashed_bytes.load(Ordering::Relaxed), cache.inner.read_jobs.load(Ordering::Relaxed), cache.inner.read_allocations.load(Ordering::Relaxed));
}

#[test]
fn inv_cache_14_index_serialization_reuses_the_output_buffer() {
    let mut text = String::with_capacity(4096);
    let entry = entry();
    let allocations = crate::alloc_probe::allocations_of(|| {
        for _ in 0..10 {
            text.clear();
            super::persist::encode_line("clip", &entry, &mut text);
        }
    });
    assert_eq!(allocations, 0, "encoding into existing capacity must not allocate");
}

#[tokio::test]
async fn inv_idle_01_failed_index_save_waits_for_an_event() {
    use std::sync::atomic::Ordering;
    let f = fixture(Limits::default(), &[]).await;
    std::fs::create_dir(f.dir.path().join(super::naming::INDEX_TMP_FILE)).unwrap();
    f.cache.put("clip", Kind::Image, b"data").await.unwrap();
    wait_until(|| f.cache.inner.persist_attempts.load(Ordering::Relaxed) > 0).await;
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert_eq!(f.cache.inner.persist_attempts.load(Ordering::Relaxed), 1);
    assert!(f.cache.inner.dirty.load(Ordering::SeqCst));
    std::fs::remove_dir(f.dir.path().join(super::naming::INDEX_TMP_FILE)).unwrap();
    f.cache.flush().await.unwrap();
    assert!(!f.cache.inner.dirty.load(Ordering::SeqCst));
    let reopened = Cache::open(f.dir.path(), Limits::default());
    reopened.ready().await;
    assert_eq!(reopened.get("clip").await.as_deref(), Some(b"data".as_slice()));
}

#[test]
fn inv_strategy_13_keep_visits_only_the_requested_family() {
    for count in [10, 1000] {
        let mut state = State::new(Limits::default());
        for number in 0..count {
            let key = format!("other/{number}");
            state.insert(super::naming::file_name(&key), key, entry(), true);
        }
        for key in ["clip", "clip#0", "clip#ns:abc#1", "clips#0"] {
            state.insert(super::naming::file_name(key), key.into(), entry(), true);
        }
        state.keep("clip");
        assert!(state.is_kept("clip") && state.is_kept("clip#0") && state.is_kept("clip#ns:abc#1"));
        assert!(!state.is_kept("clips#0"));
        assert_eq!(state.keep_visits, 3, "unrelated entries were visited at N={count}");
        let name = super::naming::file_name("clip#0");
        state.insert(name.clone(), "clip#0".into(), entry(), true);
        assert!(state.is_kept("clip#0"));
        state.take(&name);
        state.insert(name, "clip#0".into(), entry(), true);
        assert!(!state.is_kept("clip#0"));
        state.keep_visits = 0;
        state.keep("absent");
        assert_eq!(state.keep_visits, 0);
    }
}

#[tokio::test]
async fn inv_cache_08_eviction_does_not_hash_candidate_names_again() {
    let f = fixture(limits(1000, 0.8, EvictionStrategy::Lru), &[]).await;
    for key in ["a", "b", "c"] {
        f.put(key, Kind::Image, 30).await;
    }
    f.cache.keep("a");
    super::naming::NAME_HASHES.with(|count| count.set(0));
    assert_eq!(super::evict::evict_to(&f.cache.inner, 60).await, 1);
    assert_eq!(super::naming::NAME_HASHES.with(|count| count.get()), 0);
    assert!(f.cache.contains("a").await);
    assert!(!f.cache.contains("b").await);
}

#[tokio::test]
async fn inv_cache_05_verified_length_does_not_clone_metadata() {
    use std::future::Future;
    let f = fixture(Limits::default(), &[]).await;
    let mut writer = f.cache.begin_write("clip", Kind::Stream).await.unwrap().with_meta(Meta {
        content_type: Some("video/mp4".into()), total_length: Some(4),
    });
    writer.write(b"data").await.unwrap();
    writer.commit().await.unwrap();
    let allocations = crate::alloc_probe::allocations_of(|| {
        let mut future = std::pin::pin!(f.cache.len("clip"));
        assert_eq!(future.as_mut().poll(&mut std::task::Context::from_waker(std::task::Waker::noop())), std::task::Poll::Ready(Some(4)));
    });
    assert_eq!(allocations, 1, "only the file name allocates on a verified length read");
}

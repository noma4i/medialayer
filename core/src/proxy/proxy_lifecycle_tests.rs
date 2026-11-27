use super::memory_store::Failure;
use super::test_client::{get, open, read_at_least};
use super::*;
use crate::fetch::mock::Script;
use std::time::Duration;
use super::proxy_harness::*;

struct GatedFetch {
    calls: std::sync::atomic::AtomicUsize,
    gate: Arc<tokio::sync::Semaphore>,
}

#[async_trait::async_trait]
impl crate::fetch::Fetcher for GatedFetch {
    async fn fetch(&self, _: crate::fetch::Request) -> Result<crate::fetch::Response, crate::fetch::FetchError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        crate::test_signal::fire();
        let gate = self.gate.clone();
        let body = futures_util::stream::once(async move {
            gate.acquire().await.unwrap().forget();
            Ok(bytes::Bytes::from_static(&[7, 8]))
        });
        Ok(crate::fetch::Response { status: 200, headers: HashMap::from([("content-length".into(), "2".into())]), body: Box::pin(body) })
    }
}

#[tokio::test]
async fn inv_heal_rebind_keeps_old_stream_until_new_listener_is_ready() {
    let fetcher = Arc::new(GatedFetch { calls: std::sync::atomic::AtomicUsize::new(0), gate: Arc::new(tokio::sync::Semaphore::new(0)) });
    let proxy = Proxy::start(config(8), fetcher.clone(), Arc::new(super::memory_store::MemoryStore::new())).await.unwrap();
    let source = "https://o.test/stream.mp4";
    let target = path_of(&proxy.url_for(source, &HashMap::new()).unwrap());
    let old_port = proxy.port();
    let stream = tokio::spawn(async move { get(old_port, &target).await });
    assert!(eventually(|| fetcher.calls.load(Ordering::SeqCst) == 1).await);
    proxy.hold_rebind(true);
    let rebind = tokio::spawn({ let proxy = proxy.clone(); async move { proxy.restart(0).await } });
    assert!(eventually(|| proxy.rebind_entered()).await);
    assert_eq!(proxy.readiness(), Readiness::Ready);
    assert_ne!(proxy.port(), old_port);
    assert_eq!(proxy.ping().await, true);
    fetcher.gate.add_permits(1);
    assert_eq!(stream.await.unwrap().body, vec![7, 8]);
    assert_eq!(proxy.readiness(), Readiness::Ready);
    proxy.hold_rebind(false);
    rebind.await.unwrap().unwrap();
}

#[tokio::test]
async fn inv_heal_06_an_aborted_repair_leaves_the_proxy_serving() {
    let rig = rig().await;
    let epoch = rig.proxy.inner.listener_epoch.load(Ordering::SeqCst);
    let _ = rig.proxy.inner.listener.lock().await.as_ref().unwrap().shutdown.send(true);
    assert!(eventually(|| rig.proxy.readiness() == Readiness::Unhealthy).await);
    rig.proxy.hold_rebind(true);
    let repair = tokio::spawn({ let proxy = rig.proxy.clone(); async move { proxy.repair().await } });
    assert!(eventually(|| rig.proxy.rebind_entered()).await);
    repair.abort();
    rig.proxy.hold_rebind(false);
    assert!(eventually(|| rig.proxy.inner.listener_epoch.load(Ordering::SeqCst) > epoch && !rig.proxy.rebind_entered() && rig.proxy.readiness() == Readiness::Ready).await);
    rig.fetcher.route("https://o.test/heal.mp4", Script::ok(vec![7]));
    let url = rig.proxy.url_for("https://o.test/heal.mp4", &HashMap::new()).unwrap();
    assert_eq!(get(rig.proxy.port(), &path_of(&url)).await.body, vec![7]);
}

#[tokio::test(start_paused = true)]
async fn inv_heal_07_accept_errors_trigger_a_rebind() {
    let rig = rig().await;
    let epoch = rig.proxy.inner.listener_epoch.load(Ordering::SeqCst);
    rig.proxy.inner.accept_error_limit.store(1, Ordering::SeqCst);
    rig.proxy.fail_accepts();
    assert!(eventually(|| rig.proxy.inner.listener_epoch.load(Ordering::SeqCst) == epoch + 1 && !rig.proxy.rebind_entered() && rig.proxy.readiness() == Readiness::Ready).await);
    tokio::time::resume();
    assert!(rig.proxy.ping().await);
}

#[tokio::test(start_paused = true)]
async fn inv_heal_stale_accept_recovery_does_not_rebind_twice() {
    let rig = rig().await;
    let epoch = rig.proxy.inner.listener_epoch.load(Ordering::SeqCst);
    let guard = rig.proxy.inner.listener.lock().await;
    let repair = tokio::spawn({ let proxy = rig.proxy.clone(); async move { proxy.repair().await } });
    tokio::task::yield_now().await;
    rig.proxy.fail_accepts();
    assert!(eventually(|| rig.proxy.inner.accept_errors.load(Ordering::SeqCst) == 8).await);
    drop(guard);
    assert_eq!(repair.await.unwrap(), true);
    let _barrier = rig.proxy.inner.listener.lock().await;
    assert_eq!(rig.proxy.inner.listener_epoch.load(Ordering::SeqCst), epoch + 1);
    assert_eq!(rig.proxy.readiness(), Readiness::Ready);
}

#[tokio::test(start_paused = true)]
async fn inv_heal_accept_success_resets_the_error_count() {
    let rig = rig().await;
    let epoch = rig.proxy.inner.listener_epoch.load(Ordering::SeqCst);
    rig.proxy.inner.accept_budget.store(7, Ordering::SeqCst);
    rig.proxy.inner.accept_failures.send_replace(false);
    assert!(eventually(|| rig.proxy.inner.accept_errors.load(Ordering::SeqCst) == 7).await);
    tokio::time::advance(Duration::from_millis(640)).await;
    tokio::time::resume();
    assert_eq!(rig.proxy.ping().await, true);
    tokio::time::pause();
    rig.proxy.inner.accept_budget.store(7, Ordering::SeqCst);
    rig.proxy.inner.accept_failures.send_replace(false);
    assert!(eventually(|| rig.proxy.inner.accept_errors.load(Ordering::SeqCst) == 14).await);
    assert_eq!(rig.proxy.inner.listener_epoch.load(Ordering::SeqCst), epoch);
    rig.proxy.inner.accept_budget.store(1, Ordering::SeqCst);
    rig.proxy.inner.accept_failures.send_replace(false);
    assert!(eventually(|| rig.proxy.inner.listener_epoch.load(Ordering::SeqCst) == epoch + 1).await);
    assert_eq!(rig.proxy.inner.accept_errors.load(Ordering::SeqCst), 15);
}

#[tokio::test(start_paused = true)]
async fn inv_heal_failed_accept_rebind_is_unhealthy_until_repair() {
    let rig = rig().await;
    let epoch = rig.proxy.inner.listener_epoch.load(Ordering::SeqCst);
    rig.proxy.inner.fail_bind.store(true, Ordering::SeqCst);
    rig.proxy.fail_accepts();
    assert!(eventually(|| rig.proxy.readiness() == Readiness::Unhealthy).await);
    assert_eq!(rig.proxy.inner.listener_epoch.load(Ordering::SeqCst), epoch);
    rig.proxy.inner.fail_bind.store(false, Ordering::SeqCst);
    assert_eq!(rig.proxy.repair().await, true);
    assert_eq!(rig.proxy.inner.listener_epoch.load(Ordering::SeqCst), epoch + 1);
    assert_eq!(rig.proxy.readiness(), Readiness::Ready);
}

#[tokio::test(start_paused = true)]
async fn inv_heal_stopped_proxy_ignores_accept_recovery() {
    let rig = rig().await;
    let epoch = rig.proxy.inner.listener_epoch.load(Ordering::SeqCst);
    let guard = rig.proxy.inner.listener.lock().await;
    let stop = tokio::spawn({ let proxy = rig.proxy.clone(); async move { proxy.stop().await } });
    tokio::task::yield_now().await;
    rig.proxy.fail_accepts();
    assert!(eventually(|| !*rig.proxy.inner.accept_failures.borrow()).await);
    drop(guard);
    stop.await.unwrap();
    assert_eq!(rig.proxy.readiness(), Readiness::Stopped);
    assert_eq!(rig.proxy.inner.listener_epoch.load(Ordering::SeqCst), epoch);
}

#[tokio::test]
async fn inv_proxy_09_playback_starts_while_prefetch_holds_every_prefetch_slot() {
    let rig = rig_with(config(2)).await;
    rig.fetcher.route("https://o.test/low1.mp4", Script::ok(pattern(10, 0)).hanging());
    rig.fetcher.route("https://o.test/low2.mp4", Script::ok(pattern(10, 0)).hanging());
    rig.fetcher.route("https://o.test/now.mp4", Script::ok(pattern(10, 7)));
    let first = tokio::spawn({ let proxy = rig.proxy.clone(); async move { proxy.fetch_into_store("https://o.test/low1.mp4", &HashMap::new(), Priority::Prefetch, None).await } });
    assert!(eventually(|| rig.fetcher.calls_for("https://o.test/low1.mp4") == 1).await);
    let second = tokio::spawn({ let proxy = rig.proxy.clone(); async move { proxy.fetch_into_store("https://o.test/low2.mp4", &HashMap::new(), Priority::Prefetch, None).await } });
    assert!(eventually(|| rig.proxy.inner.slots.waiting() == 1).await);
    assert_eq!(rig.fetcher.calls_for("https://o.test/low2.mp4"), 0);

    let now = within(2, get(rig.proxy.port(), &path_of(&rig.proxy.url_for("https://o.test/now.mp4", &HashMap::new()).unwrap()))).await;
    assert_eq!(now.body, pattern(10, 7));
    first.abort();
    second.abort();
}

#[tokio::test]
async fn inv_image_12_fetch_reports_network_then_cache() {
    let rig = rig().await;
    let url = "https://o.test/image.png";
    let bytes = pattern(128, 3);
    rig.fetcher.route(url, Script::ok(bytes.clone()));
    let first = rig.proxy.fetch_bytes(url, &HashMap::new(), 1024).await.unwrap();
    rig.fetcher.set_offline(true);
    let second = rig.proxy.fetch_bytes(url, &HashMap::new(), 1024).await.unwrap();
    assert_eq!(first.source, super::ImageOrigin::Network);
    assert_eq!(second.source, super::ImageOrigin::Cache);
    assert_eq!(first.bytes, bytes);
    assert_eq!(second.bytes, bytes);
    assert_eq!(rig.fetcher.calls(), 1);
}

#[tokio::test]
async fn inv_proxy_03_prefetch_fills_the_store_and_later_playback_works_offline() {
    let rig = rig().await;
    rig.fetcher.route("https://o.test/p.mp4", Script::ok(pattern(5000, 6)).chunked(1000));
    rig.proxy.fetch_into_store("https://o.test/p.mp4", &HashMap::new(), Priority::Prefetch, None).await.unwrap();
    assert_eq!(rig.store.bytes("https://o.test/p.mp4"), Some(pattern(5000, 6)));

    rig.proxy.fetch_into_store("https://o.test/p.mp4", &HashMap::new(), Priority::Prefetch, None).await.unwrap();
    assert_eq!(rig.fetcher.calls(), 1);
    rig.fetcher.set_offline(true);
    let reply = get(rig.proxy.port(), &path_of(&rig.proxy.url_for("https://o.test/p.mp4", &HashMap::new()).unwrap())).await;
    assert_eq!(reply.body, pattern(5000, 6));
}

#[tokio::test]
async fn inv_proxy_08_prefetch_reports_origin_status_and_fetch_failures() {
    let rig = rig().await;
    rig.fetcher.route("https://o.test/gone.mp4", Script::ok(b"x".to_vec()).status(404));
    assert_eq!(rig.proxy.fetch_into_store("https://o.test/gone.mp4", &HashMap::new(), Priority::Prefetch, None).await, Err(ProxyError::Status(404)));
    rig.fetcher.set_offline(true);
    let failure = rig.proxy.fetch_into_store("https://o.test/other.mp4", &HashMap::new(), Priority::Prefetch, None).await;
    assert!(matches!(failure, Err(ProxyError::Fetch(_))));
    rig.fetcher.set_offline(false);
    rig.store.fail_at(Some(Failure::Begin));
    rig.fetcher.route("https://o.test/full.mp4", Script::ok(pattern(10, 0)));
    assert_eq!(rig.proxy.fetch_into_store("https://o.test/full.mp4", &HashMap::new(), Priority::Prefetch, None).await, Err(ProxyError::StoreRefused));
}

#[tokio::test]
async fn inv_proxy_17_restart_on_another_port_raises_the_route_generation() {
    let rig = rig().await;
    assert_eq!(rig.proxy.route_generation(), 0);
    let first = rig.proxy.port();
    let spare = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    assert_eq!(rig.proxy.restart(spare).await, Ok(spare));
    assert_eq!(rig.proxy.route_generation(), 1);
    assert_ne!(spare, first);

    let replacement = rig.proxy.restart(spare).await.unwrap();
    assert_ne!(replacement, spare);
    assert_eq!(rig.proxy.route_generation(), 2);

    let occupied = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let busy = occupied.local_addr().unwrap().port();
    let fallback = rig.proxy.restart(busy).await.unwrap();
    assert_ne!(fallback, busy);
    assert_eq!(rig.proxy.port(), fallback);
    assert_eq!(rig.proxy.route_generation(), 2 + u64::from(fallback != replacement));
    assert!(rig.proxy.ping().await);
}

#[tokio::test]
async fn inv_proxy_17_ping_and_readiness_follow_the_lifecycle() {
    let rig = rig().await;
    assert_eq!(rig.proxy.readiness(), Readiness::Ready);
    assert!(rig.proxy.ping().await);
    assert!(rig.proxy.url_for("https://o.test/a.mp4", &HashMap::new()).unwrap().starts_with(&format!("http://127.0.0.1:{}/", rig.proxy.port())));
    let port = rig.proxy.port();

    rig.proxy.stop().await;
    assert_eq!(rig.proxy.readiness(), Readiness::Stopped);
    assert!(!rig.proxy.ping().await);
    assert_eq!(rig.proxy.url_for("https://o.test/a.mp4", &HashMap::new()), None);
    assert!(tokio::net::TcpStream::connect(("127.0.0.1", port)).await.is_err());
    assert_eq!(rig.proxy.live_tasks(), 0);

    rig.proxy.restart(0).await.unwrap();
    assert_eq!(rig.proxy.readiness(), Readiness::Ready);
    assert!(rig.proxy.ping().await);
}

#[tokio::test]
async fn inv_proxy_23_a_listener_closed_under_the_accept_loop_is_rebound_by_repair() {
    let rig = rig().await;
    let port = rig.proxy.port();
    super::test_client::close_listener_silently(port);

    assert!(tokio::net::TcpStream::connect(("127.0.0.1", port)).await.is_err());
    assert_eq!(rig.proxy.readiness(), Readiness::Ready);
    assert!(!rig.proxy.ping().await);

    assert!(rig.proxy.repair().await);

    assert!(rig.proxy.ping().await);
    rig.fetcher.route("https://o.test/a.mp4", Script::ok(pattern(10, 3)));
    let url = rig.proxy.url_for("https://o.test/a.mp4", &HashMap::new()).unwrap();
    let reply = get(rig.proxy.port(), &path_of(&url)).await;
    assert_eq!(reply.body, pattern(10, 3));
}

#[tokio::test]
async fn inv_proxy_23_repair_leaves_a_healthy_or_stopped_proxy_alone() {
    let rig = rig().await;
    let (port, generation) = (rig.proxy.port(), rig.proxy.route_generation());
    assert!(rig.proxy.repair().await);
    assert_eq!((rig.proxy.port(), rig.proxy.route_generation()), (port, generation));

    rig.proxy.stop().await;
    assert!(!rig.proxy.repair().await);
    assert_eq!(rig.proxy.readiness(), Readiness::Stopped);
}

#[tokio::test]
async fn inv_proxy_07_stop_closes_open_connections_and_cancels_their_downloads() {
    let rig = rig().await;
    rig.fetcher.route("https://o.test/slow.mp4", Script::filled(10 * 1024 * 1024).chunked(16 * 1024).chunk_delay(Duration::from_millis(5)));
    let target = path_of(&rig.proxy.url_for("https://o.test/slow.mp4", &HashMap::new()).unwrap());
    let mut stream = open(rig.proxy.port(), "GET", &target, &[]).await;
    read_at_least(&mut stream, 32 * 1024).await;
    rig.proxy.stop().await;
    assert!(eventually(|| rig.proxy.live_tasks() == 0).await);
    assert_eq!(rig.store.open_writers(), 0);
}

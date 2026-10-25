use super::buffer::Buffers;
use super::byte_store::ByteStore;
use super::config::Config;
use super::flight::{download, Flight, FlightOutcome, FlightSubscription};
use super::route::block_key;
use super::tasks::Tasks;
use super::window::Windows;
use crate::hls::is_manifest_url;
use super::slots::{Priority, PrioritySlots};
use crate::fetch::Fetcher;
use crate::identity::namespaced_key;
use super::recent::Recent;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;
use tokio::sync::watch;
use tokio::time::Instant;
use tokio::sync::oneshot;

const MAX_STAMPS: usize = 256;
const MAX_TOTALS: usize = 1024;
const MAX_REGISTERED: usize = 4096;

pub(super) struct LiveGuard(Arc<AtomicUsize>);

impl LiveGuard {
    pub fn new(counter: &Arc<AtomicUsize>) -> Self {
        counter.fetch_add(1, Ordering::SeqCst);
        LiveGuard(counter.clone())
    }
}

impl Drop for LiveGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

pub(super) struct Listener {
    pub shutdown: watch::Sender<bool>,
    pub finished: oneshot::Receiver<()>,
}

type ProbeEntry = (Arc<tokio::sync::Mutex<()>>, usize);

pub(super) struct ProxyShared {
    pub fetcher: Arc<dyn Fetcher>,
    pub store: Arc<dyn ByteStore>,
    pub config: Config,
    pub slots: PrioritySlots,
    pub image_slots: PrioritySlots,
    pub live: Arc<AtomicUsize>,
    pub flights: Mutex<HashMap<String, Arc<Flight>>>,
    pub registered_headers: Mutex<Registry>,
    pub tasks: Tasks,
    pub windows: Windows,
    pub playlist_stamps: Mutex<Recent<Instant>>,
    pub playlists: super::playlists::Playlists,
    pub totals: Mutex<Recent<u64>>,
    whole_only: Mutex<Recent<()>>,
    probe_locks: Mutex<HashMap<String, ProbeEntry>>,
    pub listener: tokio::sync::Mutex<Option<Listener>>,
    pub running: AtomicBool,
    pub alive: Arc<AtomicBool>,
    pub port: AtomicU16,
    pub last_port: AtomicU16,
    pub route_generation: AtomicU64,
    timeout_ms: AtomicU64,
}

pub(super) struct Registry(Recent<(HashMap<String, String>, bool)>);

pub(super) struct ProbeLock<'a> {
    shared: &'a ProxyShared,
    key: &'a str,
    pub lock: Arc<tokio::sync::Mutex<()>>,
}

impl Drop for ProbeLock<'_> {
    fn drop(&mut self) {
        let mut locks = locked(&self.shared.probe_locks);
        if let Some((_, users)) = locks.get_mut(self.key) {
            *users -= 1;
            if *users == 0 {
                locks.remove(self.key);
            }
        }
    }
}

impl Registry {
    fn new() -> Self {
        Registry(Recent::new(MAX_REGISTERED))
    }

    fn get(&mut self, url: &str) -> Option<&HashMap<String, String>> {
        self.0.get(url).map(|(headers, _)| headers)
    }

    fn insert(&mut self, url: &str, headers: &HashMap<String, String>, replace: bool, planned: impl Fn(&str) -> bool) {
        let kept = match self.0.get(url) {
            Some((existing, explicit)) if !replace && *explicit => Some((existing.clone(), true)),
            _ => None,
        };
        let entry = kept.unwrap_or_else(|| (headers.clone(), replace));
        self.0.insert(url, entry, |url, _| planned(url));
    }
}

#[cfg(test)]
impl ProxyShared {
    pub fn registered_headers_of(&self, url: &str) -> Option<HashMap<String, String>> {
        locked(&self.registered_headers).get(url).cloned()
    }

    pub fn unregister_headers(&self, url: &str) {
        locked(&self.registered_headers).0.remove(url);
    }
}

pub(super) fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl ProxyShared {
    pub fn new(config: Config, fetcher: Arc<dyn Fetcher>, store: Arc<dyn ByteStore>, tasks: Tasks) -> Self {
        let timeout_ms = AtomicU64::new(config.policy.timeout.as_millis() as u64);
        let chunks_ahead = config.chunks_ahead;
        ProxyShared {
            slots: PrioritySlots::new(config.max_concurrent, config.playback_reserved, config.max_waiting),
            image_slots: PrioritySlots::new(config.max_concurrent, config.playback_reserved, config.max_waiting),
            fetcher,
            store,
            config,
            live: Arc::new(AtomicUsize::new(0)),
            flights: Mutex::new(HashMap::new()),
            registered_headers: Mutex::new(Registry::new()),
            tasks,
            windows: Windows::new(chunks_ahead),
            playlist_stamps: Mutex::new(Recent::new(MAX_STAMPS)),
            playlists: super::playlists::Playlists::default(),
            totals: Mutex::new(Recent::new(MAX_TOTALS)),
            whole_only: Mutex::new(Recent::new(MAX_TOTALS)),
            probe_locks: Mutex::new(HashMap::new()),
            listener: tokio::sync::Mutex::new(None),
            running: AtomicBool::new(false),
            alive: Arc::new(AtomicBool::new(false)),
            port: AtomicU16::new(0),
            last_port: AtomicU16::new(0),
            route_generation: AtomicU64::new(0),
            timeout_ms,
        }
    }

    pub fn gate(&self, priority: Priority) -> &PrioritySlots {
        if priority.is_image() { &self.image_slots } else { &self.slots }
    }

    pub fn timeout(&self) -> Duration {
        Duration::from_millis(self.timeout_ms.load(Ordering::SeqCst))
    }

    pub fn set_timeout(&self, timeout: Duration) {
        self.timeout_ms.store(timeout.as_millis() as u64, Ordering::SeqCst);
    }

    pub fn buffers(&self) -> Buffers {
        Buffers::new(&self.config, self.timeout())
    }

    pub fn register_headers(&self, url: &str, headers: &HashMap<String, String>) {
        locked(&self.registered_headers).insert(url, headers, true, |url| self.windows.is_planned(url));
    }

    pub fn inherit_headers(&self, urls: &[String], headers: &HashMap<String, String>) {
        let mut registry = locked(&self.registered_headers);
        for url in urls {
            registry.insert(url, headers, false, |url| self.windows.is_planned(url));
        }
    }

    pub fn upstream_headers(&self, url: &str, incoming: &HashMap<String, String>) -> HashMap<String, String> {
        let mut merged = locked(&self.registered_headers).get(url).cloned().unwrap_or_default();
        for (name, value) in incoming {
            merged.retain(|existing, _| !existing.eq_ignore_ascii_case(name));
            merged.insert(name.clone(), value.clone());
        }
        let mut forwarded = self.config.policy.forward(&merged);
        forwarded.retain(|name, _| !name.eq_ignore_ascii_case("range") && !name.eq_ignore_ascii_case("accept-encoding"));
        // Byte ranges and playlist parsing require uncompressed origin bytes.
        forwarded.insert("Accept-Encoding".to_string(), "identity".to_string());
        forwarded
    }

    pub fn key(&self, url: &str, headers: &HashMap<String, String>) -> String {
        namespaced_key(url, headers, &self.config.namespace_headers)
    }

    pub fn existing_flight(&self, url: &str, headers: &HashMap<String, String>) -> Option<FlightSubscription> {
        locked(&self.flights).get(&self.key(url, headers)).map(|flight| {
            flight.promote();
            flight.subscribe()
        })
    }

    pub fn get_or_start(self: &Arc<Self>, url: &str, headers: HashMap<String, String>, priority: Priority) -> FlightSubscription {
        let key = self.key(url, &headers);
        self.start_flight(url, key.clone(), key, None, headers, priority)
    }

    pub fn get_or_start_block(
        self: &Arc<Self>,
        url: &str,
        block: u64,
        total: Option<u64>,
        headers: HashMap<String, String>,
        priority: Priority,
    ) -> FlightSubscription {
        let size = self.config.chunk_size;
        let start = block * size;
        let end = total.map_or(start + size, |total| (start + size).min(total)) - 1;
        let base = self.key(url, &headers);
        self.start_flight(url, block_key(&base, block), base, Some((start, end)), headers, priority)
    }

    fn start_flight(
        self: &Arc<Self>,
        url: &str,
        key: String,
        base_key: String,
        range: Option<(u64, u64)>,
        headers: HashMap<String, String>,
        priority: Priority,
    ) -> FlightSubscription {
        let (flight, subscription, created) = {
            let mut flights = locked(&self.flights);
            match flights.get(&key) {
                Some(existing) => {
                    if !priority.is_prefetch() {
                        existing.promote();
                    }
                    (existing.clone(), existing.subscribe(), false)
                }
                None => {
                    let flight = Flight::new(url, &key, &base_key, range, priority);
                    flights.insert(key.clone(), flight.clone());
                    let subscription = flight.subscribe();
                    (flight, subscription, true)
                }
            }
        };
        if created {
            let guard = LiveGuard::new(&self.live);
            let inner = self.clone();
            self.tasks.spawn(async move {
                let _guard = guard;
                inner.run_flight(flight, headers, priority).await;
            });
        }
        subscription
    }

    async fn run_flight(&self, flight: Arc<Flight>, headers: HashMap<String, String>, priority: Priority) {
        crate::trace!("flight", "start {} range={:?} {priority:?}", crate::trace::tail(&flight.url), flight.range);
        let started = std::time::Instant::now();
        let outcome = download(self, &flight, headers, priority).await;
        crate::trace!("flight", "end {} range={:?} {outcome:?} bytes={} {priority:?} after={}ms", crate::trace::tail(&flight.url), flight.range, flight.written(), started.elapsed().as_millis());
        {
            let mut flights = locked(&self.flights);
            if flights.get(&flight.key).is_some_and(|current| Arc::ptr_eq(current, &flight)) {
                flights.remove(&flight.key);
            }
        }
        if matches!(outcome, FlightOutcome::Complete) && is_manifest_url(&flight.url) {
            self.stamp_playlist(&flight.key);
        }
        flight.finish(outcome);
    }

    pub fn stamp_playlist(&self, key: &str) {
        locked(&self.playlist_stamps).insert(key, Instant::now(), |_, _| false);
    }

    pub fn playlist_is_fresh(&self, key: &str) -> bool {
        locked(&self.playlist_stamps).get(key).is_some_and(|stamped| stamped.elapsed() <= self.config.playlist_ttl)
    }

    pub fn forget_playlist(&self, key: &str) {
        locked(&self.playlist_stamps).remove(key);
        self.playlists.forget(Some(key));
    }

    pub fn remember_total(&self, key: &str, total: u64) {
        locked(&self.totals).insert(key, total, |_, _| false);
    }

    pub fn mark_ignores_range(&self, key: &str) {
        locked(&self.whole_only).insert(key, (), |_, _| false);
    }

    pub fn ignores_range(&self, key: &str) -> bool {
        locked(&self.whole_only).get(key).is_some()
    }

    pub fn probe_lock<'a>(&'a self, key: &'a str) -> ProbeLock<'a> {
        let mut locks = locked(&self.probe_locks);
        let (lock, users) = locks.entry(key.to_string()).or_default();
        *users += 1;
        ProbeLock { shared: self, key, lock: lock.clone() }
    }

    #[cfg(test)]
    pub fn probe_count(&self) -> usize {
        locked(&self.probe_locks).len()
    }

    pub fn known_total(&self, key: &str) -> Option<u64> {
        locked(&self.totals).get(key).copied()
    }

    pub fn cancel_all_flights(&self) {
        for flight in locked(&self.flights).values() {
            flight.force_cancel();
        }
    }

    pub fn retire_if_abandoned(&self, flight: &Arc<Flight>) -> bool {
        let mut flights = locked(&self.flights);
        if !flight.is_abandoned() {
            return false;
        }
        if flights.get(&flight.key).is_some_and(|current| Arc::ptr_eq(current, flight)) {
            flights.remove(&flight.key);
        }
        true
    }
}

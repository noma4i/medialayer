use super::byte_store::ByteStore;
use super::config::Config;
use super::flight::{download, Flight, Outcome, Subscription};
use super::route::{authority, block_key};
use crate::hls::is_manifest_url;
use super::slots::{Priority, PrioritySlots};
use crate::fetch::Fetcher;
use crate::identity::cache_key;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use tokio::sync::watch;
use tokio::time::Instant;
use tokio::task::JoinHandle;

const MAX_STAMPS: usize = 256;
const MAX_TOTALS: usize = 1024;

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
    pub task: JoinHandle<()>,
}

pub(super) struct Inner {
    pub fetcher: Arc<dyn Fetcher>,
    pub store: Arc<dyn ByteStore>,
    pub config: Config,
    pub slots: PrioritySlots,
    pub live: Arc<AtomicUsize>,
    pub flights: Mutex<HashMap<String, Arc<Flight>>>,
    pub registered_headers: Mutex<HashMap<String, HashMap<String, String>>>,
    pub playlist_stamps: Mutex<HashMap<String, Instant>>,
    pub totals: Mutex<HashMap<String, u64>>,
    pub listener: tokio::sync::Mutex<Option<Listener>>,
    pub running: AtomicBool,
    pub alive: Arc<AtomicBool>,
    pub port: AtomicU16,
    pub last_port: AtomicU16,
    pub route_generation: AtomicU64,
}

pub(super) fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Inner {
    pub fn new(config: Config, fetcher: Arc<dyn Fetcher>, store: Arc<dyn ByteStore>) -> Self {
        Inner {
            slots: PrioritySlots::new(config.max_concurrent, config.playback_reserved, config.max_waiting),
            fetcher,
            store,
            config,
            live: Arc::new(AtomicUsize::new(0)),
            flights: Mutex::new(HashMap::new()),
            registered_headers: Mutex::new(HashMap::new()),
            playlist_stamps: Mutex::new(HashMap::new()),
            totals: Mutex::new(HashMap::new()),
            listener: tokio::sync::Mutex::new(None),
            running: AtomicBool::new(false),
            alive: Arc::new(AtomicBool::new(false)),
            port: AtomicU16::new(0),
            last_port: AtomicU16::new(0),
            route_generation: AtomicU64::new(0),
        }
    }

    pub fn register_headers(&self, url: &str, headers: &HashMap<String, String>) {
        if let (Some(host), false) = (authority(url), headers.is_empty()) {
            locked(&self.registered_headers).insert(host, headers.clone());
        }
    }

    pub fn upstream_headers(&self, url: &str, incoming: &HashMap<String, String>) -> HashMap<String, String> {
        let mut merged = authority(url)
            .and_then(|host| locked(&self.registered_headers).get(&host).cloned())
            .unwrap_or_default();
        for (name, value) in incoming {
            merged.retain(|existing, _| !existing.eq_ignore_ascii_case(name));
            merged.insert(name.clone(), value.clone());
        }
        let mut forwarded = self.config.policy.forward(&merged);
        forwarded.retain(|name, _| !name.eq_ignore_ascii_case("range"));
        forwarded
    }

    pub fn existing_flight(&self, url: &str) -> Option<Subscription> {
        locked(&self.flights).get(&cache_key(url)).map(|flight| {
            flight.promote();
            flight.subscribe()
        })
    }

    pub fn get_or_start(self: &Arc<Self>, url: &str, headers: HashMap<String, String>, priority: Priority) -> Subscription {
        self.start_flight(url, cache_key(url), None, headers, priority)
    }

    pub fn get_or_start_block(
        self: &Arc<Self>,
        url: &str,
        block: u64,
        total: u64,
        headers: HashMap<String, String>,
        priority: Priority,
    ) -> Subscription {
        let size = self.config.chunk_size;
        let start = block * size;
        let end = (start + size).min(total) - 1;
        self.start_flight(url, block_key(&cache_key(url), block), Some((start, end)), headers, priority)
    }

    fn start_flight(
        self: &Arc<Self>,
        url: &str,
        key: String,
        range: Option<(u64, u64)>,
        headers: HashMap<String, String>,
        priority: Priority,
    ) -> Subscription {
        let (flight, subscription, created) = {
            let mut flights = locked(&self.flights);
            match flights.get(&key) {
                Some(existing) => {
                    if priority == Priority::Playback {
                        existing.promote();
                    }
                    (existing.clone(), existing.subscribe(), false)
                }
                None => {
                    let flight = Flight::new(url, &key, range, priority);
                    flights.insert(key.clone(), flight.clone());
                    let subscription = flight.subscribe();
                    (flight, subscription, true)
                }
            }
        };
        if created {
            let guard = LiveGuard::new(&self.live);
            let inner = self.clone();
            tokio::spawn(async move {
                let _guard = guard;
                inner.run_flight(flight, headers, priority).await;
            });
        }
        subscription
    }

    async fn run_flight(&self, flight: Arc<Flight>, headers: HashMap<String, String>, priority: Priority) {
        let outcome = download(self, &flight, headers, priority).await;
        {
            let mut flights = locked(&self.flights);
            if flights.get(&flight.key).is_some_and(|current| Arc::ptr_eq(current, &flight)) {
                flights.remove(&flight.key);
            }
        }
        if matches!(outcome, Outcome::Complete) && is_manifest_url(&flight.url) {
            self.stamp_playlist(&flight.key);
        }
        flight.finish(outcome);
    }

    pub fn stamp_playlist(&self, key: &str) {
        let mut stamps = locked(&self.playlist_stamps);
        let ttl = self.config.playlist_ttl;
        if stamps.len() >= MAX_STAMPS {
            stamps.retain(|_, stamped| stamped.elapsed() <= ttl);
        }
        stamps.insert(key.to_string(), Instant::now());
    }

    pub fn playlist_is_fresh(&self, key: &str) -> bool {
        locked(&self.playlist_stamps).get(key).is_some_and(|stamped| stamped.elapsed() <= self.config.playlist_ttl)
    }

    pub fn forget_playlist(&self, key: &str) {
        locked(&self.playlist_stamps).remove(key);
    }

    pub fn remember_total(&self, key: &str, total: u64) {
        let mut totals = locked(&self.totals);
        if totals.len() >= MAX_TOTALS {
            totals.clear();
        }
        totals.insert(key.to_string(), total);
    }

    pub fn known_total(&self, key: &str) -> Option<u64> {
        locked(&self.totals).get(key).copied()
    }

    pub fn cancel_all_flights(&self) {
        for flight in locked(&self.flights).values() {
            flight.cancel();
        }
    }
}

use super::deferred::Deferred;
use crate::api::MediaError;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

struct Value;

fn settled(deferred: &Deferred<Value>) -> Result<Arc<Value>, MediaError> {
    let caller = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    caller.block_on(async {
        let waited = tokio::time::timeout(Duration::from_secs(2), deferred.wait()).await;
        waited.unwrap_or_else(|_| Err(MediaError::Fetch { detail: "never settled".to_string() }))
    })
}

#[test]
fn inv_ffi_12_a_value_installed_after_the_close_is_dropped_and_its_work_never_runs() {
    let deferred = Deferred::<Value>::new();
    let ran = Arc::new(AtomicUsize::new(0));
    deferred.close();
    let value = Arc::new(Value);
    let weak = Arc::downgrade(&value);

    deferred.install(value);
    let counted = ran.clone();
    deferred.run(move |_| {
        counted.fetch_add(1, Ordering::SeqCst);
    });

    assert_eq!(weak.strong_count(), 0, "the closed deferred revived the value");
    assert_eq!(ran.load(Ordering::SeqCst), 0);
    assert!(matches!(settled(&deferred), Err(MediaError::Unexpected { .. })));
}

#[test]
fn inv_ffi_12_a_failure_after_the_close_does_not_replace_the_close_outcome() {
    let deferred = Deferred::<Value>::new();

    deferred.close();
    deferred.fail(MediaError::Fetch { detail: "late".to_string() });

    assert!(matches!(settled(&deferred), Err(MediaError::Unexpected { .. })));
}

#[test]
fn inv_ffi_12_closing_releases_the_installed_value_and_every_queued_closure() {
    let ready = Deferred::<Value>::new();
    let value = Arc::new(Value);
    let weak = Arc::downgrade(&value);
    ready.install(value);
    let waiting = Deferred::<Value>::new();
    let token = Arc::new(());
    let queued = Arc::downgrade(&token);
    waiting.run(move |_| drop(token));

    ready.close();
    waiting.close();

    assert_eq!(weak.strong_count(), 0, "the closed deferred still owns the value");
    assert_eq!(queued.strong_count(), 0, "the closed deferred still owns a queued closure");
    assert!(matches!(settled(&waiting), Err(MediaError::Unexpected { .. })));
}

#[test]
fn inv_ffi_12_a_close_during_the_drain_stops_the_queue_and_keeps_the_close_outcome() {
    let deferred = Deferred::<Value>::new();
    let (inside, entered) = mpsc::channel();
    let (go, resume) = mpsc::channel::<()>();
    let ran = Arc::new(AtomicUsize::new(0));
    deferred.run(move |_| {
        inside.send(()).unwrap();
        resume.recv().unwrap();
    });
    let queued = ran.clone();
    deferred.run(move |_| {
        queued.fetch_add(1, Ordering::SeqCst);
    });
    let value = Arc::new(Value);
    let weak = Arc::downgrade(&value);
    let installer = {
        let deferred = deferred.clone();
        std::thread::spawn(move || deferred.install(value))
    };
    entered.recv().unwrap();

    deferred.close();
    let late = ran.clone();
    deferred.run(move |_| {
        late.fetch_add(1, Ordering::SeqCst);
    });
    go.send(()).unwrap();
    installer.join().unwrap();

    assert_eq!(ran.load(Ordering::SeqCst), 0, "work ran after the close");
    assert_eq!(weak.strong_count(), 0, "the drain revived the closed value");
    assert!(matches!(settled(&deferred), Err(MediaError::Unexpected { .. })));
}

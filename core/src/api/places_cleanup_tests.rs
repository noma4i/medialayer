use super::*;

#[tokio::test]
async fn inv_api_32_last_subscription_releases_idle_place() {
    let places = Places::new(Handle::current());
    let first = places.subscribe("a", Arc::new(|_| {}));
    let second = places.subscribe("a", Arc::new(|_| {}));
    drop(first);
    assert_eq!(places.entries().len(), 1);
    places.declared("a", false);
    drop(second);
    assert!(places.is_declared("a"));
    places.retired("a");
    assert!(places.entries().is_empty());

    let mut remaining = Vec::new();
    for order in 0..4 {
        let places = Places::new(Handle::current());
        for index in 0..100 {
            let key = format!("https://o.test/{index}");
            let subscription = places.subscribe(&key, Arc::new(|_| {}));
            if order != 0 {
                places.declared(&key, false);
            }
            match order {
                1 => {
                    drop(subscription);
                    assert!(places.is_declared(&key));
                    places.retired(&key);
                }
                2 => {
                    places.retired(&key);
                    drop(subscription);
                }
                3 => {
                    places.retire_all();
                    drop(subscription);
                }
                _ => drop(subscription),
            }
        }
        remaining.push(places.entries().len());
    }
    assert_eq!(remaining, vec![0; 4], "idle places remain: undeclared, drop-first, retire-first, retire-all");
}

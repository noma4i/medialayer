use super::*;
use crate::analysis::{analyze, ToneData, ToneSource};
use crate::limits::Limits;

fn tone_of(level: u8) -> ToneData {
    analyze(2, 2, &[[level, level, level, 255]; 4].concat(), ToneSource::Pixels).unwrap()
}

#[tokio::test]
async fn inv_tone_08_a_stored_tone_survives_a_restart_and_leaves_with_its_entry() {
    let dir = tempfile::tempdir().unwrap();
    let cache = Cache::open(dir.path(), Limits::default());
    cache.ready().await;
    cache.put("image-key", Kind::Image, b"first bytes").await.unwrap();

    assert!(cache.set_tone("image-key", tone_of(10)));
    assert!(!cache.set_tone("missing-key", tone_of(10)));
    assert_eq!(cache.tone("image-key"), Some(tone_of(10)));
    cache.flush().await.unwrap();
    drop(cache);

    let reopened = Cache::open(dir.path(), Limits::default());
    reopened.ready().await;
    assert_eq!(reopened.tone("image-key"), Some(tone_of(10)));
    assert!(reopened.set_tone("image-key", tone_of(200)));
    assert_eq!(reopened.tone("image-key"), Some(tone_of(200)));
    reopened.put("image-key", Kind::Image, b"second bytes").await.unwrap();
    assert_eq!(reopened.tone("image-key"), None);
    assert!(reopened.set_tone("image-key", tone_of(10)));
    assert!(reopened.remove("image-key").await);
    assert_eq!(reopened.tone("image-key"), None);
    reopened.put("image-key", Kind::Image, b"third bytes").await.unwrap();
    assert_eq!(reopened.tone("image-key"), None);
}

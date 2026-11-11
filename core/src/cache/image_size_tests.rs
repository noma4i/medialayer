use super::*;
use crate::image_size::ImageSize;
use crate::limits::Limits;

const SMALL: ImageSize = ImageSize { width: 640, height: 480 };
const LARGE: ImageSize = ImageSize { width: 4000, height: 3000 };

#[tokio::test]
async fn inv_image_44_a_stored_image_size_survives_a_restart_and_leaves_with_its_entry() {
    let dir = tempfile::tempdir().unwrap();
    let cache = Cache::open(dir.path(), Limits::default());
    cache.ready().await;
    cache.put("image-key", Kind::Image, b"first bytes").await.unwrap();

    assert!(cache.set_image_size("image-key", SMALL));
    assert!(!cache.set_image_size("missing-key", SMALL));
    assert_eq!(cache.image_size("image-key"), Some(SMALL));
    cache.flush().await.unwrap();
    drop(cache);

    let reopened = Cache::open(dir.path(), Limits::default());
    reopened.ready().await;
    assert_eq!(reopened.image_size("image-key"), Some(SMALL));
    assert!(reopened.set_image_size("image-key", LARGE));
    assert_eq!(reopened.image_size("image-key"), Some(LARGE));
    reopened.put("image-key", Kind::Image, b"second bytes").await.unwrap();
    assert_eq!(reopened.image_size("image-key"), None);
    assert!(reopened.set_image_size("image-key", SMALL));
    assert!(reopened.remove("image-key").await);
    assert_eq!(reopened.image_size("image-key"), None);
}

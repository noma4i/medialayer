use super::test_support::proxy_rig;
use super::*;
use crate::api::MediaError;
use crate::fetch::mock::Script;
use crate::image_size::fixtures::{padded, png};
use crate::image_size::ImageSize;
use crate::proxy::{block_key, BLOCK_SIZE as BLOCK};
use std::collections::HashMap;
use std::sync::Arc;
use url::Url;

const URL: &str = "https://cdn.test/photo.png";

fn image_script(length: usize) -> Script {
    Script::ok(padded(png(1200, 800), length)).header("Content-Type", "image/png")
}

#[tokio::test]
async fn inv_image_43_the_size_comes_from_one_ranged_head_fetch_and_not_from_the_whole_file() {
    let rig = proxy_rig().await;
    let routes = Routes::new(rig.proxy.clone(), Arc::new(rig.cache.clone()));
    rig.fetcher.route(URL, image_script(5 * BLOCK as usize));

    let size = routes.image_size(URL, &HashMap::new()).await.unwrap();

    assert_eq!(size, Some(ImageSize { width: 1200, height: 800 }));
    let ranges: Vec<_> = rig.fetcher.requests().into_iter().map(|request| request.range).collect();
    assert_eq!(ranges, vec![Some((0, Some(BLOCK - 1)))], "requested ranges");
    assert!(rig.fetcher.bytes_served() <= BLOCK, "served {} bytes", rig.fetcher.bytes_served());
}

#[tokio::test]
async fn inv_image_45_a_repeat_asks_no_network_and_a_cached_whole_file_asks_none_at_all() {
    let rig = proxy_rig().await;
    let routes = Routes::new(rig.proxy.clone(), Arc::new(rig.cache.clone()));
    rig.fetcher.route(URL, image_script(5 * BLOCK as usize));
    routes.image_size(URL, &HashMap::new()).await.unwrap();
    let calls = rig.fetcher.calls();
    let key = rig.proxy.key_for(URL, &HashMap::new());
    assert!(rig.cache.image_size(&block_key(&key, 0)).is_some(), "the answer is kept on the stored block");

    let again = routes.image_size(URL, &HashMap::new()).await.unwrap();

    assert_eq!(again, Some(ImageSize { width: 1200, height: 800 }));
    assert_eq!(rig.fetcher.calls(), calls, "a repeat reached the network");
    let kept = ImageSize { width: 7, height: 9 };
    assert!(rig.cache.set_image_size(&block_key(&key, 0), kept));
    assert_eq!(routes.image_size(URL, &HashMap::new()).await.unwrap(), Some(kept), "a repeat read the head again instead of the kept answer");

    let whole = "https://cdn.test/whole.png";
    rig.fetcher.route(whole, Script::ok(png(90, 60)).header("Content-Type", "image/png"));
    rig.proxy.fetch_bytes(whole, &HashMap::new(), BLOCK).await.unwrap();
    let calls = rig.fetcher.calls();

    assert_eq!(routes.image_size(whole, &HashMap::new()).await.unwrap(), Some(ImageSize { width: 90, height: 60 }));
    assert_eq!(rig.fetcher.calls(), calls, "a stored file reached the network");
}

#[tokio::test]
async fn inv_image_46_a_local_file_is_read_by_its_head_once_and_a_foreign_file_has_no_size() {
    let rig = proxy_rig().await;
    let routes = Routes::new(rig.proxy.clone(), Arc::new(rig.cache.clone()));
    let dir = tempfile::tempdir().unwrap();
    let photo = dir.path().join("photo one.png");
    std::fs::write(&photo, padded(png(30, 20), 4096)).unwrap();
    let url = Url::from_file_path(&photo).unwrap().to_string();

    assert_eq!(routes.image_size(&url, &HashMap::new()).await.unwrap(), Some(ImageSize { width: 30, height: 20 }));
    std::fs::remove_file(&photo).unwrap();
    assert_eq!(routes.image_size(&url, &HashMap::new()).await.unwrap(), Some(ImageSize { width: 30, height: 20 }));

    let text = dir.path().join("note.txt");
    std::fs::write(&text, b"not an image").unwrap();
    assert_eq!(routes.image_size(&Url::from_file_path(&text).unwrap().to_string(), &HashMap::new()).await.unwrap(), None);
    assert!(matches!(routes.image_size("file:///missing/none.png", &HashMap::new()).await, Err(MediaError::Fetch { .. })));
    assert_eq!(rig.fetcher.calls(), 0);
}

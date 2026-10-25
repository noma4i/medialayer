use super::fixtures::*;
use super::*;

fn size(width: u32, height: u32) -> Option<ImageSize> {
    Some(ImageSize { width, height })
}

#[test]
fn inv_image_42_the_head_of_png_gif_webp_and_jpeg_gives_the_pixel_size_with_the_exif_turn() {
    assert_eq!(parse(&png(640, 480)), size(640, 480));
    assert_eq!(parse(&gif(320, 200)), size(320, 200));
    assert_eq!(parse(&webp_lossy(400, 300)), size(400, 300));
    assert_eq!(parse(&webp_lossless(1024, 513)), size(1024, 513));
    assert_eq!(parse(&webp_extended(4000, 3000)), size(4000, 3000));
    assert_eq!(parse(&jpeg(800, 600, None)), size(800, 600));
    assert_eq!(parse(&jpeg(800, 600, Some((1, true)))), size(800, 600));
    assert_eq!(parse(&jpeg(800, 600, Some((6, true)))), size(600, 800));
    assert_eq!(parse(&jpeg(800, 600, Some((8, false)))), size(600, 800));
    assert_eq!(parse(&jpeg(800, 600, Some((3, false)))), size(800, 600));
}

#[test]
fn inv_image_42_a_cut_or_foreign_head_gives_no_size() {
    let jpeg_head = jpeg(800, 600, Some((6, true)));
    for (head, needed) in [(png(640, 480), 24), (gif(320, 200), 10), (webp_extended(4000, 3000), 30), (jpeg_head.clone(), jpeg_head.len())] {
        assert!(parse(&head[..needed]).is_some(), "{needed} bytes are enough");
        for length in 0..needed {
            assert_eq!(parse(&head[..length]), None, "{length} of {needed} bytes");
        }
    }
    assert_eq!(parse(b"%PDF-1.7 not an image at all"), None);
    assert_eq!(parse(&png(0, 480)), None);
    assert_eq!(ImageSize::decode(&ImageSize { width: 7, height: 9 }.encode()), size(7, 9));
    assert_eq!(ImageSize::decode("7x"), None);
    assert_eq!(ImageSize::decode("0x9"), None);
}

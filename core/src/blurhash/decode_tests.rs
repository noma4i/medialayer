use super::*;

const KNOWN: &str = "LEHV6nWB2yk8pyo0adR*.7kCMdnj";

fn pixel(data: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let at = ((y * width + x) * 4) as usize;
    [data[at], data[at + 1], data[at + 2], data[at + 3]]
}

fn channel_sum(data: &[u8]) -> u64 {
    data.iter().map(|byte| *byte as u64).sum()
}

#[test]
fn components_of_known_hash() {
    assert_eq!(components(KNOWN), Ok((4, 3)));
}

#[test]
fn components_of_single_component_hash() {
    assert_eq!(components("00TI:j"), Ok((1, 1)));
}

#[test]
fn components_reject_short_hash() {
    assert_eq!(components("LEHV6"), Err(BlurHashError::TooShort));
    assert_eq!(components(""), Err(BlurHashError::TooShort));
}

#[test]
fn components_reject_out_of_range_count() {
    assert_eq!(components("~~~~~~"), Err(BlurHashError::ComponentsOutOfRange));
}

#[test]
fn known_hash_corner_and_center_pixels() {
    let data = decode(KNOWN, 32, 32, 1.0).unwrap();
    assert_eq!(data.len(), 32 * 32 * 4);
    assert_eq!(pixel(&data, 32, 0, 0), [135, 164, 177, 255]);
    assert_eq!(pixel(&data, 32, 31, 0), [137, 166, 181, 255]);
    assert_eq!(pixel(&data, 32, 0, 31), [136, 144, 147, 255]);
    assert_eq!(pixel(&data, 32, 31, 31), [133, 142, 147, 255]);
    assert_eq!(pixel(&data, 32, 16, 16), [158, 125, 108, 255]);
    assert_eq!(pixel(&data, 32, 8, 24), [148, 139, 133, 255]);
    assert_eq!(channel_sum(&data), 719147);
}

#[test]
fn known_hash_average_color_is_dc_component() {
    let data = decode(KNOWN, 32, 32, 1.0).unwrap();
    let mut totals = [0u64; 3];
    for chunk in data.chunks(4) {
        for channel in 0..3 {
            totals[channel] += chunk[channel] as u64;
        }
    }
    let average: Vec<i64> = totals.iter().map(|sum| (*sum / 1024) as i64).collect();
    assert!((average[0] - 151).abs() <= 12);
    assert!((average[1] - 150).abs() <= 12);
    assert!((average[2] - 149).abs() <= 12);
}

#[test]
fn known_hash_alpha_is_opaque_everywhere() {
    let data = decode(KNOWN, 32, 32, 1.0).unwrap();
    assert!(data.chunks(4).all(|chunk| chunk[3] == 255));
}

#[test]
fn punch_changes_saturation() {
    let data = decode(KNOWN, 32, 32, 2.0).unwrap();
    assert_eq!(pixel(&data, 32, 0, 0), [116, 176, 201, 255]);
    assert_eq!(pixel(&data, 32, 16, 16), [165, 92, 0, 255]);
    assert_eq!(pixel(&data, 32, 31, 31), [110, 133, 145, 255]);
    assert_eq!(channel_sum(&data), 705241);
}

#[test]
fn single_component_hash_is_one_flat_color() {
    let data = decode("00TI:j", 5, 7, 1.0).unwrap();
    assert_eq!(data.len(), 5 * 7 * 4);
    assert!(data.chunks(4).all(|chunk| chunk == [255, 0, 0, 255]));
}

#[test]
fn single_component_hash_ignores_punch() {
    let data = decode("005?}k", 3, 3, 4.0).unwrap();
    assert!(data.chunks(4).all(|chunk| chunk == [51, 102, 153, 255]));
}

#[test]
fn other_sizes_keep_known_corner_and_length() {
    let data = decode(KNOWN, 17, 9, 1.0).unwrap();
    assert_eq!(data.len(), 17 * 9 * 4);
    assert_eq!(pixel(&data, 17, 0, 0), [135, 164, 177, 255]);
    assert_eq!(pixel(&data, 17, 16, 8), [133, 142, 148, 255]);
    assert_eq!(channel_sum(&data), 107810);
}

#[test]
fn one_pixel_image_is_the_top_left_corner() {
    let data = decode(KNOWN, 1, 1, 1.0).unwrap();
    assert_eq!(data.len(), 4);
    assert_eq!(&data[..], &[135, 164, 177, 255]);
}

#[test]
fn rejects_short_hash() {
    assert_eq!(decode("00TI:", 4, 4, 1.0), Err(BlurHashError::TooShort));
    assert_eq!(decode("", 4, 4, 1.0), Err(BlurHashError::TooShort));
}

#[test]
fn rejects_length_for_wrong_component_count() {
    let longer = format!("{KNOWN}0");
    assert_eq!(decode(&longer, 4, 4, 1.0), Err(BlurHashError::LengthMismatch));
    assert_eq!(decode(&KNOWN[..KNOWN.len() - 1], 4, 4, 1.0), Err(BlurHashError::LengthMismatch));
}

#[test]
fn rejects_character_outside_base83() {
    let bad = "LEHV6nWB2yk8pyo0adR*.7kCMdn ";
    assert_eq!(decode(bad, 4, 4, 1.0), Err(BlurHashError::InvalidCharacter));
    assert_eq!(decode("00TI:\"", 4, 4, 1.0), Err(BlurHashError::InvalidCharacter));
}

#[test]
fn rejects_non_ascii_without_panic() {
    assert!(decode("00TI:\u{e9}\u{e9}", 4, 4, 1.0).is_err());
    assert!(decode("\u{1f600}\u{1f600}\u{1f600}", 4, 4, 1.0).is_err());
}

#[test]
fn rejects_zero_dimensions() {
    assert_eq!(decode(KNOWN, 0, 4, 1.0), Err(BlurHashError::ZeroSize));
    assert_eq!(decode(KNOWN, 4, 0, 1.0), Err(BlurHashError::ZeroSize));
}

#[test]
fn rejects_component_count_above_nine() {
    assert_eq!(decode("~~~~~~", 4, 4, 1.0), Err(BlurHashError::ComponentsOutOfRange));
}

#[test]
fn rejects_oversized_image_without_allocating() {
    assert_eq!(decode(KNOWN, u32::MAX, u32::MAX, 1.0), Err(BlurHashError::SizeTooLarge));
}

#[test]
fn rejects_non_finite_punch() {
    assert_eq!(decode(KNOWN, 4, 4, f32::NAN), Err(BlurHashError::InvalidPunch));
    assert_eq!(decode(KNOWN, 4, 4, f32::INFINITY), Err(BlurHashError::InvalidPunch));
}

#[test]
fn decode_is_deterministic() {
    assert_eq!(decode(KNOWN, 8, 8, 1.0), decode(KNOWN, 8, 8, 1.0));
}

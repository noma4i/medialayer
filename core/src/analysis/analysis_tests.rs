use super::*;
use crate::alloc_probe::allocations_of;
use crate::blurhash::decode;

const KNOWN: &str = "LEHV6nWB2yk8pyo0adR*.7kCMdnj";
const WHITE: [u8; 3] = [255, 255, 255];
const BLACK: [u8; 3] = [0, 0, 0];
const TOLERANCE: f64 = 1e-9;
const BLURHASH_CHANNEL_TOLERANCE: i32 = 2;
const DOMINANT_GAP: i32 = 40;

fn rgb(channels: [u8; 3]) -> Rgb {
    Rgb { r: channels[0], g: channels[1], b: channels[2] }
}

fn paint(width: u32, height: u32, color_at: impl Fn(u32, u32) -> [u8; 3]) -> Vec<u8> {
    (0..height)
        .flat_map(|y| (0..width).map(move |x| (x, y)))
        .flat_map(|(x, y)| {
            let [r, g, b] = color_at(x, y);
            [r, g, b, 255]
        })
        .collect()
}

fn analyzed(width: u32, height: u32, rgba: &[u8]) -> ToneData {
    analyze(width, height, rgba, ToneSource::Pixels).unwrap()
}

fn channel_gap(left: Rgb, right: Rgb) -> i32 {
    [(left.r, right.r), (left.g, right.g), (left.b, right.b)].iter().map(|(a, b)| (i32::from(*a) - i32::from(*b)).abs()).max().unwrap()
}

#[test]
fn inv_tone_01_solid_images_give_exact_cells_overall_and_luminance() {
    for (channels, expected) in [(WHITE, 1.0), (BLACK, 0.0)] {
        let tone = analyzed(16, 16, &paint(16, 16, |_, _| channels)).at(0.15).unwrap();

        assert_eq!(tone.source, ToneSource::Pixels);
        assert_eq!((tone.columns as usize, tone.rows as usize), (COLUMNS, ROWS));
        assert_eq!(tone.cells, vec![rgb(channels); COLUMNS * ROWS]);
        for swatch in [tone.overall, tone.top, tone.rest] {
            assert_eq!(swatch.color, rgb(channels));
            assert!((swatch.luminance - expected).abs() < TOLERANCE, "{swatch:?}");
        }
    }
}

#[test]
fn inv_tone_01_a_white_top_over_a_black_bottom_gives_a_white_top_band_and_a_dark_rest() {
    let rgba = paint(100, 100, |_, y| if y < 30 { WHITE } else { BLACK });
    let data = analyzed(100, 100, &rgba);

    let tone = data.at(0.15).unwrap();

    assert_eq!(tone.top.color, rgb(WHITE));
    assert!((tone.top.luminance - 1.0).abs() < TOLERANCE);
    assert!((tone.rest.luminance - 0.179).abs() < 0.01, "{:?}", tone.rest);
    assert_eq!(tone, data.at(0.15).unwrap());
}

#[test]
fn inv_tone_01_a_one_pixel_image_fills_every_cell_and_the_only_dominant_colour() {
    let tone = analyzed(1, 1, &[10, 20, 30, 255]).at(0.15).unwrap();

    assert_eq!(tone.cells, vec![Rgb { r: 10, g: 20, b: 30 }; COLUMNS * ROWS]);
    assert_eq!(tone.overall.color, Rgb { r: 10, g: 20, b: 30 });
    assert_eq!(tone.dominant.len(), 1);
    assert!((tone.dominant[0].share - 1.0).abs() < TOLERANCE);
}

#[test]
fn inv_tone_02_invalid_input_is_an_error_and_the_pixel_limit_is_exact() {
    let cases: [(u32, u32, usize, ToneError); 6] = [
        (0, 0, 0, ToneError::ZeroSize),
        (0, 4, 0, ToneError::ZeroSize),
        (4, 0, 0, ToneError::ZeroSize),
        (2, 2, 15, ToneError::LengthMismatch),
        (2, 2, 17, ToneError::LengthMismatch),
        (129, 128, 129 * 128 * 4, ToneError::TooLarge { pixels: 129 * 128, limit: 128 * 128 }),
    ];
    for (width, height, length, expected) in cases {
        assert_eq!(analyze(width, height, &vec![0; length], ToneSource::Pixels), Err(expected), "{width}x{height} {length}");
    }

    assert!(analyze(128, 128, &vec![0; 128 * 128 * 4], ToneSource::Pixels).is_ok());
    assert!(analyze_blurhash("short").is_err());
}

#[test]
fn inv_tone_03_the_band_is_a_parameter_of_the_query_and_a_wider_band_reaches_further_down() {
    let rgba = paint(40, 120, |_, y| {
        let level = (y * 255 / 119) as u8;
        [level; 3]
    });
    let data = analyzed(40, 120, &rgba);

    let luminances: Vec<f64> = [0.15, 0.5, 1.0].iter().map(|band| data.at(*band).unwrap().top.luminance).collect();

    assert!(luminances[0] < luminances[1] && luminances[1] < luminances[2], "{luminances:?}");
    let whole = data.at(1.0).unwrap();
    assert_eq!(whole.top, whole.rest);
    assert!(channel_gap(whole.top.color, whole.overall.color) <= 1);
    assert_eq!(data.at(0.3).unwrap().band, 0.3);
    for band in [0.0, -0.1, 1.01, f32::NAN, f32::INFINITY] {
        assert_eq!(data.at(band), Err(ToneError::InvalidBand), "{band}");
    }
}

#[test]
fn inv_tone_04_dominant_colours_are_at_most_3_distinct_and_ordered_by_share() {
    let share = |tone: &MediaTone| -> Vec<(Rgb, f64)> { tone.dominant.iter().map(|dominant| (dominant.color, dominant.share)).collect() };
    let stripes = |height: u32, limits: [(u32, [u8; 3]); 4]| {
        paint(10, height, move |_, y| limits.iter().find(|(until, _)| y < *until).map_or(BLACK, |(_, color)| *color))
    };
    let red = [255, 0, 0];
    let green = [0, 255, 0];
    let blue = [0, 0, 255];
    let yellow = [255, 255, 0];

    let three = analyzed(10, 100, &stripes(100, [(60, red), (90, green), (100, blue), (100, blue)])).at(0.15).unwrap();
    let four = analyzed(10, 100, &stripes(100, [(40, red), (70, green), (90, blue), (100, yellow)])).at(0.15).unwrap();
    let solid = analyzed(4, 4, &paint(4, 4, |_, _| green)).at(0.15).unwrap();

    let ordered = share(&three);
    assert_eq!(ordered.iter().map(|(color, _)| *color).collect::<Vec<_>>(), vec![rgb(red), rgb(green), rgb(blue)]);
    assert!((ordered[0].1 - 0.6).abs() < 0.002 && (ordered[1].1 - 0.3).abs() < 0.002 && (ordered[2].1 - 0.1).abs() < 0.002, "{ordered:?}");
    assert_eq!(four.dominant.len(), DOMINANT);
    assert_eq!(four.dominant[0].color, rgb(red));
    assert_eq!(share(&solid), vec![(rgb(green), 1.0)]);
    let gradient = analyzed(256, 4, &paint(256, 4, |x, _| [x as u8; 3])).at(0.15).unwrap();
    assert!(gradient.dominant.len() <= DOMINANT);
    for (index, left) in gradient.dominant.iter().enumerate() {
        for right in &gradient.dominant[index + 1..] {
            assert!(channel_gap(left.color, right.color) >= DOMINANT_GAP, "{left:?} {right:?}");
        }
    }
}

#[test]
fn inv_tone_05_a_blurhash_and_the_pixels_it_decodes_to_agree_within_the_stated_tolerance_and_name_their_source() {
    let from_hash = analyze_blurhash(KNOWN).unwrap().at(0.15).unwrap();
    let decoded = decode(KNOWN, 64, 48, 1.0).unwrap();
    let from_pixels = analyze(64, 48, &decoded, ToneSource::Pixels).unwrap().at(0.15).unwrap();

    assert_eq!(from_hash.source, ToneSource::BlurHash);
    assert_eq!(from_pixels.source, ToneSource::Pixels);
    for (hash, pixels) in [(from_hash.top, from_pixels.top), (from_hash.rest, from_pixels.rest), (from_hash.overall, from_pixels.overall)] {
        assert!(channel_gap(hash.color, pixels.color) <= BLURHASH_CHANNEL_TOLERANCE, "{hash:?} {pixels:?}");
    }
}

#[test]
fn inv_tone_06_the_stored_form_round_trips_and_every_damaged_form_is_refused() {
    let gradient = paint(30, 30, |x, y| [(x * 8) as u8, (y * 8) as u8, 40]);
    let forms = [
        analyzed(30, 30, &gradient),
        analyzed(4, 4, &paint(4, 4, |_, _| WHITE)),
        analyze_blurhash(KNOWN).unwrap(),
    ];
    for form in forms {
        let bytes = form.encode();
        assert_eq!(ToneData::decode(&bytes), Some(form.clone()));
        for length in 0..bytes.len() {
            assert_eq!(ToneData::decode(&bytes[..length]), None, "cut at {length}");
        }
        let mut longer = bytes.clone();
        longer.push(0);
        assert_eq!(ToneData::decode(&longer), None);
        let mut other_version = bytes.clone();
        other_version[0] ^= 0xff;
        assert_eq!(ToneData::decode(&other_version), None);
        let mut other_source = bytes.clone();
        other_source[1] = 9;
        assert_eq!(ToneData::decode(&other_source), None);
    }
}

#[test]
fn inv_tone_07_the_analysis_allocations_do_not_depend_on_the_pixel_count() {
    let small = paint(16, 16, |x, y| [x as u8, y as u8, 7]);
    let large = paint(128, 128, |x, y| [x as u8, y as u8, 7]);
    analyzed(16, 16, &small);

    let few = allocations_of(|| analyze(16, 16, &small, ToneSource::Pixels));
    let many = allocations_of(|| analyze(128, 128, &large, ToneSource::Pixels));

    assert_eq!(few, many);
}

use super::data::{DominantColor, ToneData, ToneError, ToneSource};
use super::swatch::{encode, linear, Rgb};
use crate::blurhash::decode;
use crate::memory::{bounded, InputLimits};

pub const COLUMNS: usize = 8;
pub const ROWS: usize = 12;
pub const DOMINANT: usize = 3;
pub const DOMINANT_GAP: i32 = 40;
pub const BLURHASH_SIDE: u32 = 32;

const BIN_BITS: u32 = 3;
const BINS: usize = 1 << (3 * BIN_BITS);
const SHARE_SCALE: u64 = 1000;

#[derive(Clone, Copy, Default)]
struct Bin {
    count: u64,
    sum: [u64; 3],
}

pub fn analyze(width: u32, height: u32, rgba: &[u8], source: ToneSource) -> Result<ToneData, ToneError> {
    if width == 0 || height == 0 {
        return Err(ToneError::ZeroSize);
    }
    let pixels = u64::from(width) * u64::from(height);
    let limit = InputLimits::default().tone_pixels;
    bounded(pixels, limit).map_err(|_| ToneError::TooLarge { pixels, limit })?;
    if rgba.len() as u64 != pixels * 4 {
        return Err(ToneError::LengthMismatch);
    }
    let (width, height) = (width as usize, height as usize);
    let mut cells = Vec::with_capacity(COLUMNS * ROWS);
    for row in 0..ROWS {
        for column in 0..COLUMNS {
            cells.push(cell_color(rgba, width, span(column, COLUMNS, width), span(row, ROWS, height)));
        }
    }
    let (overall, dominant) = summarize(rgba, pixels);
    Ok(ToneData { source, cells, overall, dominant })
}

pub fn analyze_blurhash(hash: &str) -> Result<ToneData, ToneError> {
    let pixels = decode(hash, BLURHASH_SIDE, BLURHASH_SIDE, 1.0).map_err(|error| ToneError::BlurHash(error.to_string()))?;
    analyze(BLURHASH_SIDE, BLURHASH_SIDE, &pixels, ToneSource::BlurHash)
}

fn span(index: usize, count: usize, size: usize) -> (usize, usize) {
    let start = index * size / count;
    (start, ((index + 1) * size / count).max(start + 1))
}

fn cell_color(rgba: &[u8], width: usize, (x0, x1): (usize, usize), (y0, y1): (usize, usize)) -> Rgb {
    let mut sum = [0.0; 3];
    for y in y0..y1 {
        for pixel in rgba[(y * width + x0) * 4..(y * width + x1) * 4].as_chunks::<4>().0 {
            for (total, channel) in sum.iter_mut().zip(pixel) {
                *total += linear(*channel);
            }
        }
    }
    let count = ((x1 - x0) * (y1 - y0)) as f64;
    encode(sum.map(|total| total / count))
}

fn summarize(rgba: &[u8], pixels: u64) -> (Rgb, Vec<DominantColor>) {
    let mut bins = vec![Bin::default(); BINS];
    let mut sum = [0.0; 3];
    for pixel in rgba.as_chunks::<4>().0 {
        let bin = &mut bins[bin_of(pixel)];
        bin.count += 1;
        for ((total, part), channel) in sum.iter_mut().zip(bin.sum.iter_mut()).zip(pixel) {
            *total += linear(*channel);
            *part += u64::from(*channel);
        }
    }
    let overall = encode(sum.map(|total| total / pixels as f64));
    let mut order: Vec<usize> = Vec::with_capacity(BINS);
    order.extend((0..BINS).filter(|bin| bins[*bin].count > 0));
    order.sort_unstable_by(|left, right| bins[*right].count.cmp(&bins[*left].count).then(left.cmp(right)));
    let mut dominant: Vec<DominantColor> = Vec::with_capacity(DOMINANT);
    for index in order {
        if dominant.len() == DOMINANT {
            break;
        }
        let bin = bins[index];
        let color = mean_color(bin);
        if dominant.iter().all(|kept| gap(kept.color, color) >= DOMINANT_GAP) {
            dominant.push(DominantColor { color, share_permille: ((bin.count * SHARE_SCALE + pixels / 2) / pixels) as u16 });
        }
    }
    (overall, dominant)
}

fn bin_of(pixel: &[u8]) -> usize {
    let shift = 8 - BIN_BITS;
    ((usize::from(pixel[0] >> shift) << (2 * BIN_BITS)) | (usize::from(pixel[1] >> shift) << BIN_BITS)) | usize::from(pixel[2] >> shift)
}

fn mean_color(bin: Bin) -> Rgb {
    let mean = |total: u64| ((total + bin.count / 2) / bin.count) as u8;
    Rgb { r: mean(bin.sum[0]), g: mean(bin.sum[1]), b: mean(bin.sum[2]) }
}

fn gap(left: Rgb, right: Rgb) -> i32 {
    [(left.r, right.r), (left.g, right.g), (left.b, right.b)].iter().map(|(a, b)| (i32::from(*a) - i32::from(*b)).abs()).max().unwrap_or(0)
}

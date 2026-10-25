use crate::memory::{bounded, InputLimits};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BlurHashError {
    #[error("blurhash is longer than the input limit")]
    TooLong,
    #[error("blurhash is shorter than 6 characters")]
    TooShort,
    #[error("blurhash length does not match its component count")]
    LengthMismatch,
    #[error("blurhash has a character outside base83")]
    InvalidCharacter,
    #[error("blurhash component count is outside 1..9")]
    ComponentsOutOfRange,
    #[error("image width and height must be above zero")]
    ZeroSize,
    #[error("image size is too large")]
    SizeTooLarge,
    #[error("punch must be a finite number")]
    InvalidPunch,
}

const ALPHABET: &[u8; 83] =
    b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz#$%*+,-.:;=?@[]^_{|}~";
const MIN_LENGTH: usize = 6;
const MAX_COMPONENTS: u32 = 9;

struct Header {
    columns: u32,
    rows: u32,
}

fn base83_value(digits: &[u8]) -> u32 {
    digits.iter().fold(0, |total, digit| {
        let index = ALPHABET.iter().position(|letter| letter == digit).unwrap_or(0);
        total * 83 + index as u32
    })
}

fn parse_header(hash: &str) -> Result<Header, BlurHashError> {
    let bytes = hash.as_bytes();
    bounded(bytes.len() as u64, InputLimits::default().blurhash_chars).map_err(|_| BlurHashError::TooLong)?;
    if bytes.len() < MIN_LENGTH {
        return Err(BlurHashError::TooShort);
    }
    if bytes.iter().any(|byte| !ALPHABET.contains(byte)) {
        return Err(BlurHashError::InvalidCharacter);
    }
    let size_flag = base83_value(&bytes[..1]);
    let columns = size_flag % 9 + 1;
    let rows = size_flag / 9 + 1;
    if columns > MAX_COMPONENTS || rows > MAX_COMPONENTS {
        return Err(BlurHashError::ComponentsOutOfRange);
    }
    if bytes.len() != 4 + 2 * (columns * rows) as usize {
        return Err(BlurHashError::LengthMismatch);
    }
    Ok(Header { columns, rows })
}

pub(crate) fn srgb_to_linear(value: u32) -> f64 {
    let scaled = value as f64 / 255.0;
    if scaled <= 0.04045 {
        scaled / 12.92
    } else {
        ((scaled + 0.055) / 1.055).powf(2.4)
    }
}

pub(crate) fn linear_to_srgb(value: f64) -> u8 {
    let clamped = value.clamp(0.0, 1.0);
    let scaled = if clamped <= 0.0031308 {
        clamped * 12.92
    } else {
        1.055 * clamped.powf(1.0 / 2.4) - 0.055
    };
    (scaled * 255.0 + 0.5) as u8
}

fn dc_color(value: u32) -> [f64; 3] {
    [
        srgb_to_linear(value >> 16),
        srgb_to_linear((value >> 8) & 255),
        srgb_to_linear(value & 255),
    ]
}

fn signed_square(value: f64) -> f64 {
    value.abs().powi(2).copysign(value)
}

fn ac_color(value: u32, scale: f64) -> [f64; 3] {
    let channel = |quantized: u32| signed_square((quantized as f64 - 9.0) / 9.0) * scale;
    [channel(value / 361), channel(value / 19 % 19), channel(value % 19)]
}

pub fn components(hash: &str) -> Result<(u32, u32), BlurHashError> {
    parse_header(hash).map(|header| (header.columns, header.rows))
}

pub fn decode(hash: &str, width: u32, height: u32, punch: f32) -> Result<Vec<u8>, BlurHashError> {
    decode_within(hash, width, height, punch, InputLimits::default().response_bytes)
}

pub(super) fn decode_within(hash: &str, width: u32, height: u32, punch: f32, budget_bytes: u64) -> Result<Vec<u8>, BlurHashError> {
    let header = parse_header(hash)?;
    if width == 0 || height == 0 {
        return Err(BlurHashError::ZeroSize);
    }
    if !punch.is_finite() {
        return Err(BlurHashError::InvalidPunch);
    }
    let columns = header.columns as usize;
    let rows = header.rows as usize;
    let byte_count = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(BlurHashError::SizeTooLarge)?;
    let horizontal_len = (width as usize).checked_mul(columns).ok_or(BlurHashError::SizeTooLarge)?;
    let vertical_len = (height as usize).checked_mul(rows).ok_or(BlurHashError::SizeTooLarge)?;
    let working_bytes = [
        byte_count,
        horizontal_len.checked_mul(size_of::<f64>()).ok_or(BlurHashError::SizeTooLarge)?,
        vertical_len.checked_mul(size_of::<f64>()).ok_or(BlurHashError::SizeTooLarge)?,
        (columns * rows) * size_of::<[f64; 3]>(),
    ]
    .into_iter()
    .try_fold(0usize, usize::checked_add)
    .ok_or(BlurHashError::SizeTooLarge)?;
    if working_bytes as u64 > budget_bytes {
        return Err(BlurHashError::SizeTooLarge);
    }
    let mut output = reserve::<u8>(byte_count)?;
    let mut colors = reserve::<[f64; 3]>(columns * rows)?;
    let mut horizontal = reserve::<f64>(horizontal_len)?;
    let mut vertical = reserve::<f64>(vertical_len)?;

    let bytes = hash.as_bytes();
    let scale = (base83_value(&bytes[1..2]) as f64 + 1.0) / 166.0 * punch as f64;
    colors.extend((0..columns * rows).map(|index| {
        if index == 0 {
            dc_color(base83_value(&bytes[2..6]))
        } else {
            ac_color(base83_value(&bytes[4 + index * 2..6 + index * 2]), scale)
        }
    }));
    fill_cosines(&mut horizontal, width, columns);
    fill_cosines(&mut vertical, height, rows);

    for vertical_row in vertical.chunks_exact(rows) {
        for horizontal_row in horizontal.chunks_exact(columns) {
            let mut sum = [0.0; 3];
            for (index, color) in colors.iter().enumerate() {
                let basis = horizontal_row[index % columns] * vertical_row[index / columns];
                for (total, part) in sum.iter_mut().zip(color) {
                    *total += part * basis;
                }
            }
            output.extend_from_slice(&[
                linear_to_srgb(sum[0]),
                linear_to_srgb(sum[1]),
                linear_to_srgb(sum[2]),
                255,
            ]);
        }
    }
    Ok(output)
}

fn reserve<T>(count: usize) -> Result<Vec<T>, BlurHashError> {
    let mut buffer = Vec::new();
    buffer.try_reserve_exact(count).map_err(|_| BlurHashError::SizeTooLarge)?;
    Ok(buffer)
}

fn fill_cosines(table: &mut Vec<f64>, size: u32, count: usize) {
    table.extend((0..size).flat_map(|position| {
        (0..count).map(move |index| (std::f64::consts::PI * position as f64 * index as f64 / size as f64).cos())
    }));
}

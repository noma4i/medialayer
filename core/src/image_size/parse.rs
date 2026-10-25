use super::exif::{orientation, turns_quarter};
use super::ImageSize;

pub fn parse(bytes: &[u8]) -> Option<ImageSize> {
    let size = match bytes {
        [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, ..] => png(bytes),
        [b'G', b'I', b'F', b'8', ..] => gif(bytes),
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => webp(bytes),
        [0xff, 0xd8, ..] => jpeg(bytes),
        _ => None,
    }?;
    (size.width > 0 && size.height > 0).then_some(size)
}

fn be32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn le16(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?)))
}

fn le24(bytes: &[u8], at: usize) -> Option<u32> {
    let part = bytes.get(at..at + 3)?;
    Some(u32::from(part[0]) | u32::from(part[1]) << 8 | u32::from(part[2]) << 16)
}

fn png(bytes: &[u8]) -> Option<ImageSize> {
    (bytes.get(12..16)? == b"IHDR").then_some(())?;
    Some(ImageSize { width: be32(bytes, 16)?, height: be32(bytes, 20)? })
}

fn gif(bytes: &[u8]) -> Option<ImageSize> {
    Some(ImageSize { width: le16(bytes, 6)?, height: le16(bytes, 8)? })
}

fn webp(bytes: &[u8]) -> Option<ImageSize> {
    match bytes.get(12..16)? {
        b"VP8 " => {
            (bytes.get(23..26)? == [0x9d, 0x01, 0x2a]).then_some(())?;
            Some(ImageSize { width: le16(bytes, 26)? & 0x3fff, height: le16(bytes, 28)? & 0x3fff })
        }
        b"VP8L" => {
            (*bytes.get(20)? == 0x2f).then_some(())?;
            let packed = u32::from_le_bytes(bytes.get(21..25)?.try_into().ok()?);
            Some(ImageSize { width: (packed & 0x3fff) + 1, height: ((packed >> 14) & 0x3fff) + 1 })
        }
        b"VP8X" => Some(ImageSize { width: le24(bytes, 24)? + 1, height: le24(bytes, 27)? + 1 }),
        _ => None,
    }
}

fn jpeg(bytes: &[u8]) -> Option<ImageSize> {
    let mut at = 2;
    let mut turn = 1;
    loop {
        while *bytes.get(at)? != 0xff { at += 1; }
        while *bytes.get(at + 1)? == 0xff { at += 1; }
        let marker = *bytes.get(at + 1)?;
        at += 2;
        match marker {
            0x01 | 0xd0..=0xd7 => continue,
            0xd9 | 0xda => return None,
            _ => {}
        }
        let length = usize::from(u16::from_be_bytes(bytes.get(at..at + 2)?.try_into().ok()?));
        let segment = bytes.get(at + 2..at + length)?;
        match marker {
            0xe1 if turn == 1 => turn = orientation(segment),
            0xc0..=0xcf if !matches!(marker, 0xc4 | 0xc8 | 0xcc) => {
                let size = ImageSize { height: u32::from(u16::from_be_bytes(segment.get(1..3)?.try_into().ok()?)), width: u32::from(u16::from_be_bytes(segment.get(3..5)?.try_into().ok()?)) };
                return Some(if turns_quarter(turn) { size.swapped() } else { size });
            }
            _ => {}
        }
        at += length;
    }
}

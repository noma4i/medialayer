const ORIENTATION_TAG: u16 = 0x0112;
const ENTRY_BYTES: usize = 12;

pub fn orientation(segment: &[u8]) -> u16 {
    segment.strip_prefix(b"Exif\0\0").and_then(read_orientation).unwrap_or(1)
}

fn read_orientation(tiff: &[u8]) -> Option<u16> {
    let little = match tiff.get(..2)? {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let short = |at: usize| tiff.get(at..at + 2).map(|pair| if little { u16::from_le_bytes([pair[0], pair[1]]) } else { u16::from_be_bytes([pair[0], pair[1]]) });
    let long = |at: usize| tiff.get(at..at + 4).map(|quad| if little { u32::from_le_bytes([quad[0], quad[1], quad[2], quad[3]]) } else { u32::from_be_bytes([quad[0], quad[1], quad[2], quad[3]]) });
    let first = long(4)? as usize;
    (0..usize::from(short(first)?)).find_map(|index| {
        let entry = first + 2 + index * ENTRY_BYTES;
        (short(entry)? == ORIENTATION_TAG).then(|| short(entry + 8)).flatten()
    })
}

pub fn turns_quarter(orientation: u16) -> bool {
    (5..=8).contains(&orientation)
}

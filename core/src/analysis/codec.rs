use super::data::{DominantColor, ToneData, ToneSource};
use super::grid::{COLUMNS, DOMINANT, ROWS};
use super::swatch::Rgb;

const VERSION: u8 = 1;
const HEADER: usize = 4;
const SHARE_BYTES: usize = 2;
const DOMINANT_BYTES: usize = 3 + SHARE_BYTES;
const MAX_SHARE: u16 = 1000;

impl ToneData {
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(HEADER + 3 * (self.cells.len() + 1) + 1 + DOMINANT_BYTES * self.dominant.len());
        bytes.extend([VERSION, source_code(self.source), COLUMNS as u8, ROWS as u8]);
        for color in self.cells.iter().chain([&self.overall]) {
            bytes.extend([color.r, color.g, color.b]);
        }
        bytes.push(self.dominant.len() as u8);
        for kept in &self.dominant {
            bytes.extend([kept.color.r, kept.color.g, kept.color.b]);
            bytes.extend(kept.share_permille.to_be_bytes());
        }
        bytes
    }

    pub fn decode(bytes: &[u8]) -> Option<ToneData> {
        let (header, body) = bytes.split_at_checked(HEADER)?;
        let source = source_of(header[1])?;
        if header[0] != VERSION || usize::from(header[2]) != COLUMNS || usize::from(header[3]) != ROWS {
            return None;
        }
        let (colors, tail) = body.split_at_checked(3 * (COLUMNS * ROWS + 1))?;
        let (count, shares) = tail.split_first()?;
        if usize::from(*count) > DOMINANT || shares.len() != DOMINANT_BYTES * usize::from(*count) {
            return None;
        }
        let mut cells: Vec<Rgb> = colors.chunks_exact(3).map(|part| Rgb { r: part[0], g: part[1], b: part[2] }).collect();
        let overall = cells.pop()?;
        let dominant = shares
            .chunks_exact(DOMINANT_BYTES)
            .map(|part| DominantColor { color: Rgb { r: part[0], g: part[1], b: part[2] }, share_permille: u16::from_be_bytes([part[3], part[4]]) })
            .collect::<Vec<_>>();
        dominant.iter().all(|kept| kept.share_permille <= MAX_SHARE).then_some(ToneData { source, cells, overall, dominant })
    }
}

fn source_code(source: ToneSource) -> u8 {
    match source {
        ToneSource::BlurHash => 0,
        ToneSource::Pixels => 1,
    }
}

fn source_of(code: u8) -> Option<ToneSource> {
    [ToneSource::BlurHash, ToneSource::Pixels].into_iter().find(|source| source_code(*source) == code)
}

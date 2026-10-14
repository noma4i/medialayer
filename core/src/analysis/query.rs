use super::data::{MediaTone, ToneData, ToneError};
use super::grid::{COLUMNS, ROWS};
use super::swatch::{encode, linear, luminance, swatch, Dominant, Swatch};

const MIN_WEIGHT: f64 = 1e-9;

pub fn valid_band(band: f32) -> Result<(), ToneError> {
    if band.is_finite() && band > 0.0 && band <= 1.0 {
        return Ok(());
    }
    Err(ToneError::InvalidBand)
}

impl ToneData {
    pub fn at(&self, band: f32) -> Result<MediaTone, ToneError> {
        valid_band(band)?;
        let covered = |row: usize| f64::from((band * ROWS as f32 - row as f32).clamp(0.0, 1.0));
        let top = self.weighted(covered).ok_or(ToneError::InvalidBand)?;
        let rest = self.weighted(|row| 1.0 - covered(row)).unwrap_or(top);
        Ok(MediaTone {
            source: self.source,
            columns: COLUMNS as u32,
            rows: ROWS as u32,
            cells: self.cells.clone(),
            band,
            overall: swatch(self.overall),
            top,
            rest,
            dominant: self
                .dominant
                .iter()
                .map(|kept| Dominant { color: kept.color, luminance: luminance(kept.color), share: f64::from(kept.share_permille) / 1000.0 })
                .collect(),
        })
    }

    fn weighted(&self, weight: impl Fn(usize) -> f64) -> Option<Swatch> {
        let mut sum = [0.0; 3];
        let mut total = 0.0;
        for (row, cells) in self.cells.chunks_exact(COLUMNS).enumerate() {
            let share = weight(row);
            for cell in cells {
                sum[0] += share * linear(cell.r);
                sum[1] += share * linear(cell.g);
                sum[2] += share * linear(cell.b);
                total += share;
            }
        }
        (total > MIN_WEIGHT).then(|| swatch(encode(sum.map(|part| part / total))))
    }
}

//! The one-time import of a pending transfer saved by DBlayer 6.43 with typed columns.

use std::collections::HashMap;
use std::str::FromStr;

use super::{media_prepare_descriptor, BitrateStep, CannotDecode, CropFractions, ImageFormat, MediaPrepareIntent, PhotoEdit, VideoLimits};
use crate::ffi::PixelSize;

type Row = HashMap<String, Option<String>>;

/// The `sync_upload_prepare` row of a saved transfer and the rows of its `sync_upload_prepare_step` table, every value as canonical text and `None` as SQL NULL.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SavedPreparation {
    pub columns: HashMap<String, Option<String>>,
    pub children: Vec<HashMap<String, Option<String>>>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum PrepareImportOutcome {
    Done { descriptor: Vec<u8> },
    Failed { message: String },
}

fn refuse(detail: String) -> String {
    format!("prepare import: {detail}")
}

fn text<'a>(columns: &'a Row, name: &str) -> Result<&'a str, String> {
    columns.get(name).and_then(Option::as_deref).ok_or_else(|| refuse(format!("the column {name} is missing or NULL")))
}

fn value<T: FromStr>(columns: &Row, name: &str) -> Result<T, String> {
    text(columns, name)?.trim().parse().map_err(|_| refuse(format!("the column {name} has a value that cannot be read")))
}

fn flag(columns: &Row, name: &str) -> Result<bool, String> {
    match text(columns, name)?.trim() {
        "0" | "false" => Ok(false),
        "1" | "true" => Ok(true),
        _ => Err(refuse(format!("the column {name} is not a flag"))),
    }
}

/// A column the old row leaves NULL.
fn optional<T: FromStr>(columns: &Row, name: &str) -> Result<Option<T>, String> {
    columns.get(name).and_then(Option::as_ref).map(|_| value(columns, name)).transpose()
}

fn size(columns: &Row, width: &str, height: &str) -> Result<Option<PixelSize>, String> {
    Ok(optional::<u32>(columns, width)?.map(|width_value| value(columns, height).map(|height| PixelSize { width: width_value, height })).transpose()?)
}

fn corridors(children: &[Row]) -> Result<Vec<BitrateStep>, String> {
    children.iter().map(|row| Ok(BitrateStep { from_long_side: value(row, "from_long_side")?, min_bps: value(row, "min_bps")?, max_bps: value(row, "max_bps")? })).collect()
}

fn video_intent(columns: &Row, children: &[Row]) -> Result<MediaPrepareIntent, String> {
    let limits = match optional::<u32>(columns, "max_long_side")? {
        None => None,
        Some(max_long_side) => Some(VideoLimits {
            max_long_side,
            corridors: corridors(children)?,
            min_frame_rate: value(columns, "min_frame_rate")?,
            max_frame_rate: value(columns, "max_frame_rate")?,
            source_bitrate_cap_percent: value(columns, "source_bitrate_cap_percent")?,
            audio_bitrate: value(columns, "audio_bitrate")?,
            audio_channels: value(columns, "audio_channels")?,
            audio_sample_rate: value(columns, "audio_sample_rate")?,
            recode_above_bytes: optional(columns, "recode_above_bytes")?,
        }),
    };
    let on_cannot_decode = match text(columns, "on_cannot_decode")? {
        "fail" => CannotDecode::Fail,
        "upload_source" => CannotDecode::UploadSource,
        other => return Err(refuse(format!("on_cannot_decode `{other}` is unknown"))),
    };
    Ok(MediaPrepareIntent::Video { limits, mirror: flag(columns, "mirror")?, on_cannot_decode })
}

fn image_intent(columns: &Row) -> Result<MediaPrepareIntent, String> {
    let crop = match optional::<f64>(columns, "crop_x")? {
        None => None,
        Some(x) => Some(CropFractions { x, y: value(columns, "crop_y")?, width: value(columns, "crop_width")?, height: value(columns, "crop_height")? }),
    };
    let format = match text(columns, "format")? {
        "jpeg" => ImageFormat::Jpeg,
        "png" => ImageFormat::Png,
        other => return Err(refuse(format!("format `{other}` is unknown"))),
    };
    let edit = PhotoEdit {
        max_size: size(columns, "max_width", "max_height")?,
        enlarge: flag(columns, "enlarge")?,
        crop,
        output_size: size(columns, "output_width", "output_height")?,
        mirror: flag(columns, "mirror")?,
        rotation_degrees: value(columns, "rotation_degrees")?,
    };
    Ok(MediaPrepareIntent::Image { edit, quality: optional(columns, "quality")?, format })
}

/// The descriptor of a pending transfer saved by DBlayer 6.43, from its typed rows: a missing or NULL required column, an unknown value or a broken step is `Failed`.
/// It exists for the D-32 upgrade of saved transfers only.
#[uniffi::export]
pub fn media_prepare_import_descriptor(saved: SavedPreparation) -> PrepareImportOutcome {
    let columns = &saved.columns;
    let intent = text(columns, "kind").and_then(|kind| match kind {
        "video" => video_intent(columns, &saved.children),
        "image" => image_intent(columns),
        other => Err(refuse(format!("kind `{other}` is unknown"))),
    });
    match intent {
        Ok(intent) => PrepareImportOutcome::Done { descriptor: media_prepare_descriptor(intent) },
        Err(message) => PrepareImportOutcome::Failed { message },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(pairs: &[(&str, Option<&str>)]) -> Row {
        pairs.iter().map(|(name, value)| (name.to_string(), value.map(str::to_string))).collect()
    }

    fn saved(columns: &[(&str, Option<&str>)], children: &[Row]) -> SavedPreparation {
        SavedPreparation { columns: row(columns), children: children.to_vec() }
    }

    fn done(intent: MediaPrepareIntent) -> PrepareImportOutcome {
        PrepareImportOutcome::Done { descriptor: media_prepare_descriptor(intent) }
    }

    fn step(from: &str, min: &str, max: &str) -> Row {
        row(&[("from_long_side", Some(from)), ("min_bps", Some(min)), ("max_bps", Some(max))])
    }

    #[test]
    fn inv_send_61_a_saved_video_row_imports_to_the_descriptor_of_the_same_intent_with_and_without_limits() {
        let none = saved(&[("kind", Some("video")), ("mirror", Some("1")), ("on_cannot_decode", Some("upload_source")), ("max_long_side", None), ("recode_above_bytes", None)], &[]);
        assert_eq!(media_prepare_import_descriptor(none), done(MediaPrepareIntent::Video { limits: None, mirror: true, on_cannot_decode: CannotDecode::UploadSource }));
        let with = saved(
            &[
                ("kind", Some("video")), ("mirror", Some("0")), ("on_cannot_decode", Some("fail")), ("max_long_side", Some("1280")), ("min_frame_rate", Some("24")), ("max_frame_rate", Some("30")),
                ("source_bitrate_cap_percent", Some("80")), ("audio_bitrate", Some("96000")), ("audio_channels", Some("2")), ("audio_sample_rate", Some("44100")), ("recode_above_bytes", Some("7000000")),
            ],
            &[step("0", "500000", "900000"), step("720", "900000", "2000000")],
        );
        let limits = VideoLimits {
            max_long_side: 1280,
            corridors: vec![BitrateStep { from_long_side: 0, min_bps: 500_000, max_bps: 900_000 }, BitrateStep { from_long_side: 720, min_bps: 900_000, max_bps: 2_000_000 }],
            min_frame_rate: 24,
            max_frame_rate: 30,
            source_bitrate_cap_percent: 80,
            audio_bitrate: 96_000,
            audio_channels: 2,
            audio_sample_rate: 44_100,
            recode_above_bytes: Some(7_000_000),
        };
        assert_eq!(media_prepare_import_descriptor(with.clone()), done(MediaPrepareIntent::Video { limits: Some(limits.clone()), mirror: false, on_cannot_decode: CannotDecode::Fail }));
        let mut null_threshold = with;
        null_threshold.columns.insert("recode_above_bytes".to_string(), None);
        assert_eq!(media_prepare_import_descriptor(null_threshold), done(MediaPrepareIntent::Video { limits: Some(VideoLimits { recode_above_bytes: None, ..limits }), mirror: false, on_cannot_decode: CannotDecode::Fail }));
    }

    #[test]
    fn inv_send_62_a_saved_image_row_imports_to_the_descriptor_of_the_same_intent() {
        let columns = [
            ("kind", Some("image")), ("mirror", Some("1")), ("max_width", Some("800")), ("max_height", Some("600")), ("enlarge", Some("1")), ("crop_x", Some("0.1")), ("crop_y", Some("0.2")),
            ("crop_width", Some("0.5")), ("crop_height", Some("0.25")), ("output_width", Some("64")), ("output_height", Some("64")), ("rotation_degrees", Some("270")), ("quality", Some("0.75")), ("format", Some("png")),
        ];
        let edit = PhotoEdit {
            max_size: Some(PixelSize { width: 800, height: 600 }),
            enlarge: true,
            crop: Some(CropFractions { x: 0.1, y: 0.2, width: 0.5, height: 0.25 }),
            output_size: Some(PixelSize { width: 64, height: 64 }),
            mirror: true,
            rotation_degrees: 270,
        };
        assert_eq!(media_prepare_import_descriptor(saved(&columns, &[])), done(MediaPrepareIntent::Image { edit, quality: Some(0.75), format: ImageFormat::Png }));
        let nulls = saved(&[("kind", Some("image")), ("mirror", Some("0")), ("enlarge", Some("0")), ("rotation_degrees", Some("0")), ("format", Some("jpeg")), ("crop_x", None), ("quality", None), ("max_width", None)], &[]);
        let plain = PhotoEdit { max_size: None, enlarge: false, crop: None, output_size: None, mirror: false, rotation_degrees: 0 };
        assert_eq!(media_prepare_import_descriptor(nulls), done(MediaPrepareIntent::Image { edit: plain, quality: None, format: ImageFormat::Jpeg }));
    }

    #[test]
    fn inv_send_63_a_missing_null_or_unreadable_column_fails_the_import() {
        let failed = |saved| matches!(media_prepare_import_descriptor(saved), PrepareImportOutcome::Failed { .. });
        let video = [("kind", Some("video")), ("mirror", Some("0")), ("on_cannot_decode", Some("fail"))];
        assert!(!failed(saved(&video, &[])));
        for name in ["kind", "mirror", "on_cannot_decode"] {
            let mut missing = saved(&video, &[]);
            missing.columns.remove(name);
            assert!(failed(missing), "{name}");
            let mut null = saved(&video, &[]);
            null.columns.insert(name.to_string(), None);
            assert!(failed(null), "{name} NULL");
        }
        assert!(failed(saved(&[("kind", Some("video")), ("mirror", Some("0")), ("on_cannot_decode", Some("fail")), ("max_long_side", Some("1280"))], &[])));
        let broken_step = saved(&video, &[row(&[("from_long_side", Some("0")), ("min_bps", None), ("max_bps", Some("1"))])]);
        let mut limited = broken_step;
        limited.columns.insert("max_long_side".to_string(), Some("1280".to_string()));
        assert!(failed(limited));
        let image = [("kind", Some("image")), ("mirror", Some("0")), ("enlarge", Some("0")), ("rotation_degrees", Some("0")), ("format", Some("gif"))];
        assert!(failed(saved(&image, &[])));
        assert!(failed(saved(&[("kind", Some("audio"))], &[])));
        assert!(failed(saved(&[], &[])));
    }
}

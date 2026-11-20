//! The one-time import of a pending transfer saved by DBlayer 6.43 with typed columns.

use std::collections::HashMap;
use std::str::FromStr;

use super::{media_prepare_descriptor, BitrateStep, CannotDecode, CropFractions, ImageFormat, MediaPrepareIntent, PhotoEdit, VideoLimits};
use crate::api::MediaError;
use crate::ffi::PixelSize;

fn refuse(detail: String) -> MediaError {
    MediaError::InvalidSpec { detail: format!("prepare import: {detail}") }
}

fn text<'a>(columns: &'a HashMap<String, String>, name: &str) -> Result<&'a str, MediaError> {
    columns.get(name).map(String::as_str).ok_or_else(|| refuse(format!("the column {name} is missing")))
}

fn value<T: FromStr>(columns: &HashMap<String, String>, name: &str) -> Result<T, MediaError> {
    text(columns, name)?.trim().parse().map_err(|_| refuse(format!("the column {name} has a value that cannot be read")))
}

fn flag(columns: &HashMap<String, String>, name: &str) -> Result<bool, MediaError> {
    match text(columns, name)?.trim() {
        "0" | "false" => Ok(false),
        "1" | "true" => Ok(true),
        _ => Err(refuse(format!("the column {name} is not a flag"))),
    }
}

/// A column the old row leaves empty is absent from the map or holds an empty text.
fn optional<T: FromStr>(columns: &HashMap<String, String>, name: &str) -> Result<Option<T>, MediaError> {
    columns.get(name).filter(|text| !text.is_empty()).map(|_| value(columns, name)).transpose()
}

fn size(columns: &HashMap<String, String>, width: &str, height: &str) -> Result<Option<PixelSize>, MediaError> {
    Ok(optional::<u32>(columns, width)?.map(|width_value| value(columns, height).map(|height| PixelSize { width: width_value, height })).transpose()?)
}

fn corridors(columns: &HashMap<String, String>) -> Result<Vec<BitrateStep>, MediaError> {
    let Some(text) = columns.get("corridors").filter(|text| !text.is_empty()) else { return Ok(Vec::new()) };
    text.split(';')
        .map(|step| {
            let parts = step.split(',').map(|part| part.trim().parse::<u32>().ok()).collect::<Option<Vec<_>>>();
            match parts.as_deref() {
                Some(&[from_long_side, min_bps, max_bps]) => Ok(BitrateStep { from_long_side, min_bps, max_bps }),
                _ => Err(refuse("the corridors are not `from,min,max` steps divided by `;`".to_string())),
            }
        })
        .collect()
}

fn video_intent(columns: &HashMap<String, String>) -> Result<MediaPrepareIntent, MediaError> {
    let limits = match optional::<u32>(columns, "max_long_side")? {
        None => None,
        Some(max_long_side) => Some(VideoLimits {
            max_long_side,
            corridors: corridors(columns)?,
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

fn image_intent(columns: &HashMap<String, String>) -> Result<MediaPrepareIntent, MediaError> {
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

/// The descriptor of a pending transfer saved by DBlayer 6.43, from the values of its `sync_upload_prepare` row read as text
/// (flags are `0`/`1`, an empty column is absent or empty text) and the `corridors` of its `sync_upload_prepare_step` rows as `from_long_side,min_bps,max_bps` steps divided by `;`.
/// A missing column or a value that cannot be read is an error. It exists for the D-32 upgrade of saved transfers only.
#[uniffi::export]
pub fn media_prepare_descriptor_import(columns: HashMap<String, String>) -> Result<Vec<u8>, MediaError> {
    let intent = match text(&columns, "kind")? {
        "video" => video_intent(&columns)?,
        "image" => image_intent(&columns)?,
        other => return Err(refuse(format!("kind `{other}` is unknown"))),
    };
    Ok(media_prepare_descriptor(intent))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn columns(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(name, value)| (name.to_string(), value.to_string())).collect()
    }

    #[test]
    fn inv_send_61_a_saved_video_row_imports_to_the_descriptor_of_the_same_intent_with_and_without_limits() {
        let none = columns(&[("kind", "video"), ("mirror", "1"), ("on_cannot_decode", "upload_source")]);
        assert_eq!(media_prepare_descriptor_import(none), Ok(media_prepare_descriptor(MediaPrepareIntent::Video { limits: None, mirror: true, on_cannot_decode: CannotDecode::UploadSource })));
        let with = columns(&[
            ("kind", "video"), ("mirror", "0"), ("on_cannot_decode", "fail"), ("max_long_side", "1280"), ("min_frame_rate", "24"), ("max_frame_rate", "30"), ("source_bitrate_cap_percent", "80"),
            ("audio_bitrate", "96000"), ("audio_channels", "2"), ("audio_sample_rate", "44100"), ("recode_above_bytes", "7000000"), ("corridors", "0,500000,900000;720,900000,2000000"),
        ]);
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
        assert_eq!(media_prepare_descriptor_import(with), Ok(media_prepare_descriptor(MediaPrepareIntent::Video { limits: Some(limits), mirror: false, on_cannot_decode: CannotDecode::Fail })));
    }

    #[test]
    fn inv_send_62_a_saved_image_row_imports_to_the_descriptor_of_the_same_intent() {
        let row = columns(&[
            ("kind", "image"), ("mirror", "1"), ("max_width", "800"), ("max_height", "600"), ("enlarge", "1"), ("crop_x", "0.1"), ("crop_y", "0.2"), ("crop_width", "0.5"), ("crop_height", "0.25"),
            ("output_width", "64"), ("output_height", "64"), ("rotation_degrees", "270"), ("quality", "0.75"), ("format", "png"),
        ]);
        let edit = PhotoEdit {
            max_size: Some(PixelSize { width: 800, height: 600 }),
            enlarge: true,
            crop: Some(CropFractions { x: 0.1, y: 0.2, width: 0.5, height: 0.25 }),
            output_size: Some(PixelSize { width: 64, height: 64 }),
            mirror: true,
            rotation_degrees: 270,
        };
        assert_eq!(media_prepare_descriptor_import(row), Ok(media_prepare_descriptor(MediaPrepareIntent::Image { edit, quality: Some(0.75), format: ImageFormat::Png })));
    }

    #[test]
    fn inv_send_63_a_missing_or_unreadable_column_fails_the_import() {
        let video = columns(&[("kind", "video"), ("mirror", "0"), ("on_cannot_decode", "fail")]);
        for name in ["kind", "mirror", "on_cannot_decode"] {
            let mut row = video.clone();
            row.remove(name);
            assert!(media_prepare_descriptor_import(row).is_err(), "{name}");
        }
        let mut limited = video.clone();
        limited.insert("max_long_side".to_string(), "1280".to_string());
        assert!(media_prepare_descriptor_import(limited).is_err());
        let image = columns(&[("kind", "image"), ("mirror", "0"), ("enlarge", "0"), ("rotation_degrees", "0"), ("format", "jpeg")]);
        assert!(media_prepare_descriptor_import(image.clone()).is_ok());
        let mut broken = image.clone();
        broken.insert("format".to_string(), "gif".to_string());
        assert!(media_prepare_descriptor_import(broken).is_err());
        assert!(media_prepare_descriptor_import(columns(&[("kind", "audio")])).is_err());
        assert!(media_prepare_descriptor_import(HashMap::new()).is_err());
    }
}

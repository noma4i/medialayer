//! The prepare descriptor: the intent of the app as opaque bytes that a queue stores, and the one decision that turns the bytes and the probe facts of a source into a plan.
//! Layout, version 1, big endian: byte 0 = 1, byte 1 = kind (0 video, 1 image), then the fields of the intent in the order of `MediaPrepareIntent`;
//! an option is a byte 0 or 1 followed by its value, a bool is a byte 0 or 1, a list is a u32 count followed by its items, an enum is its index.

use super::{check, photo_plan, plan_video_sized, video_metadata_to_strip, BitrateStep, CropFractions, ImageFormat, PhotoEdit, PhotoPlan, VideoLimits, VideoPlan, VideoSource};
use crate::ffi::PixelSize;

const VERSION: u8 = 1;

/// What happens to a source the device cannot decode: the prepare fails with `CannotDecode`, or the source goes as it is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, uniffi::Enum)]
pub enum CannotDecode {
    #[default]
    Fail,
    UploadSource,
}

/// What the app asks of a file before it is sent. `limits = None` takes `default_video_limits`. `max_bytes` is the largest result that may be sent (D-412), `None` has no limit.
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum MediaPrepareIntent {
    Video { limits: Option<VideoLimits>, mirror: bool, on_cannot_decode: CannotDecode, max_bytes: Option<u64> },
    Image { edit: PhotoEdit, quality: Option<f64>, format: ImageFormat, max_bytes: Option<u64> },
}

/// What the platform read from a source. `size_bytes` is the file size.
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum PrepareProbe {
    Video { source: VideoSource, metadata_keys: Vec<String>, size_bytes: u64 },
    Image { width: u32, height: u32, exif_orientation: u32, format: ImageFormat },
    Failed { message: String },
}

/// The whole job of the platform writer: it decides nothing.
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum PreparePlan {
    Video { plan: VideoPlan, strip_metadata: Vec<String>, audio_bitrate: u32, audio_channels: u32, audio_sample_rate: u32, mirror: bool },
    Image { plan: PhotoPlan, quality: f64, format: ImageFormat },
}

/// Why a prepare ends without a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum MediaPrepareFailure {
    CannotDecode,
    SourceUnreadable,
    WriteFailed,
    OverMaxBytes,
    UnsupportedDescriptor,
}

/// How a prepare ends. `Done.path` is the destination.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum MediaPrepareOutcome {
    Done { path: String, mime: String, size_bytes: u64, width: u32, height: u32 },
    Failed { reason: MediaPrepareFailure, message: String },
    Cancelled,
}

/// The planned result of a prepare, with the turn, the mirror and the crop applied, also when the source goes as it is.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MediaPrepareFacts {
    pub width: u32,
    pub height: u32,
    pub mime: String,
}

/// What the platform does: copy the source to the destination as it is, run the plan, or end with the refusal.
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum PrepareDecision {
    Pass,
    Run { plan: PreparePlan },
    Refuse { reason: MediaPrepareFailure, message: String },
}

#[uniffi::export]
pub fn media_prepare_descriptor(intent: MediaPrepareIntent) -> Vec<u8> {
    let mut out = vec![VERSION];
    match intent {
        MediaPrepareIntent::Video { limits, mirror, on_cannot_decode, max_bytes } => {
            out.push(0);
            put_option(&mut out, limits, |out, limits| {
                put_u32(out, limits.max_long_side);
                put_u32(out, limits.corridors.len() as u32);
                for step in &limits.corridors {
                    [step.from_long_side, step.min_bps, step.max_bps].into_iter().for_each(|value| put_u32(out, value));
                }
                [limits.min_frame_rate, limits.max_frame_rate, limits.source_bitrate_cap_percent, limits.audio_bitrate, limits.audio_channels, limits.audio_sample_rate].into_iter().for_each(|value| put_u32(out, value));
                put_option(out, limits.recode_above_bytes, |out, bytes| out.extend(bytes.to_be_bytes()));
            });
            out.push(u8::from(mirror));
            out.push(on_cannot_decode as u8);
            put_option(&mut out, max_bytes, |out, bytes| out.extend(bytes.to_be_bytes()));
        }
        MediaPrepareIntent::Image { edit, quality, format, max_bytes } => {
            out.push(1);
            put_option(&mut out, edit.max_size, put_size);
            out.push(u8::from(edit.enlarge));
            put_option(&mut out, edit.crop, |out, crop| [crop.x, crop.y, crop.width, crop.height].into_iter().for_each(|value| out.extend(value.to_be_bytes())));
            put_option(&mut out, edit.output_size, put_size);
            out.push(u8::from(edit.mirror));
            put_u32(&mut out, edit.rotation_degrees);
            put_option(&mut out, quality, |out, quality| out.extend(quality.to_be_bytes()));
            out.push(format as u8);
            put_option(&mut out, max_bytes, |out, bytes| out.extend(bytes.to_be_bytes()));
        }
    }
    out
}

/// The intent in `bytes`. An unknown version, a short or a longer input and an unknown tag are `UnsupportedDescriptor`.
pub(crate) fn decode_descriptor(bytes: &[u8]) -> Result<MediaPrepareIntent, MediaPrepareFailure> {
    let mut reader = Reader { bytes };
    let intent = read_intent(&mut reader).ok_or(MediaPrepareFailure::UnsupportedDescriptor)?;
    reader.bytes.is_empty().then_some(intent).ok_or(MediaPrepareFailure::UnsupportedDescriptor)
}

fn read_intent(reader: &mut Reader) -> Option<MediaPrepareIntent> {
    if reader.u8()? != VERSION {
        return None;
    }
    match reader.u8()? {
        0 => {
            let limits = reader.option(|reader| {
                let max_long_side = reader.u32()?;
                let count = reader.u32()? as usize;
                if count > reader.bytes.len() / 12 {
                    return None;
                }
                let corridors = (0..count).map(|_| Some(BitrateStep { from_long_side: reader.u32()?, min_bps: reader.u32()?, max_bps: reader.u32()? })).collect::<Option<Vec<_>>>()?;
                Some(VideoLimits {
                    max_long_side,
                    corridors,
                    min_frame_rate: reader.u32()?,
                    max_frame_rate: reader.u32()?,
                    source_bitrate_cap_percent: reader.u32()?,
                    audio_bitrate: reader.u32()?,
                    audio_channels: reader.u32()?,
                    audio_sample_rate: reader.u32()?,
                    recode_above_bytes: reader.option(|reader| Some(u64::from_be_bytes(reader.take()?)))?,
                })
            })?;
            let mirror = reader.bool()?;
            let on_cannot_decode = match reader.u8()? {
                0 => CannotDecode::Fail,
                1 => CannotDecode::UploadSource,
                _ => return None,
            };
            let max_bytes = reader.option(|reader| Some(u64::from_be_bytes(reader.take()?)))?;
            Some(MediaPrepareIntent::Video { limits, mirror, on_cannot_decode, max_bytes })
        }
        1 => {
            let max_size = reader.option(Reader::size)?;
            let enlarge = reader.bool()?;
            let crop = reader.option(|reader| Some(CropFractions { x: reader.f64()?, y: reader.f64()?, width: reader.f64()?, height: reader.f64()? }))?;
            let output_size = reader.option(Reader::size)?;
            let mirror = reader.bool()?;
            let rotation_degrees = reader.u32()?;
            let quality = reader.option(Reader::f64)?;
            let format = match reader.u8()? {
                0 => ImageFormat::Jpeg,
                1 => ImageFormat::Png,
                _ => return None,
            };
            let max_bytes = reader.option(|reader| Some(u64::from_be_bytes(reader.take()?)))?;
            Some(MediaPrepareIntent::Image { edit: PhotoEdit { max_size, enlarge, crop, output_size, mirror, rotation_degrees }, quality, format, max_bytes })
        }
        _ => None,
    }
}

const VIDEO_MIME: &str = "video/mp4";

fn refuse(reason: MediaPrepareFailure, message: impl Into<String>) -> PrepareDecision {
    PrepareDecision::Refuse { reason, message: message.into() }
}

/// The decision of a prepare: the descriptor and what the platform read from the source give a pass, a plan or a refusal.
/// A source that cannot be read, a kind that does not match the intent and an image that has no plan are the cannot decode case: `UploadSource` passes the original, `Fail` refuses.
#[uniffi::export]
pub fn media_prepare_decide(descriptor: Vec<u8>, probe: PrepareProbe) -> PrepareDecision {
    let intent = match decode_descriptor(&descriptor) {
        Ok(intent) => intent,
        Err(reason) => return refuse(reason, "the prepare descriptor has an unknown version or is broken"),
    };
    if let Err(message) = check_settings(&intent) {
        return refuse(MediaPrepareFailure::UnsupportedDescriptor, message);
    }
    let cannot_decode = || match &intent {
        MediaPrepareIntent::Video { on_cannot_decode: CannotDecode::UploadSource, .. } => PrepareDecision::Pass,
        _ => refuse(MediaPrepareFailure::CannotDecode, "the device cannot decode the source"),
    };
    match (&intent, &probe) {
        (MediaPrepareIntent::Video { limits, mirror, .. }, PrepareProbe::Video { source, metadata_keys, size_bytes }) => {
            let limits = limits.clone().unwrap_or_else(super::default_video_limits);
            match plan_video_sized(*source, *size_bytes, limits.clone()) {
                Err(error) => refuse(MediaPrepareFailure::UnsupportedDescriptor, error.to_string()),
                Ok(plan) => {
                    let strip_metadata = video_metadata_to_strip(metadata_keys.clone());
                    if !plan.reencode && strip_metadata.is_empty() && !mirror {
                        return PrepareDecision::Pass;
                    }
                    PrepareDecision::Run { plan: PreparePlan::Video { plan, strip_metadata, audio_bitrate: limits.audio_bitrate, audio_channels: limits.audio_channels, audio_sample_rate: limits.audio_sample_rate, mirror: *mirror } }
                }
            }
        }
        (MediaPrepareIntent::Image { edit, quality, format, .. }, PrepareProbe::Image { width, height, exif_orientation, .. }) => photo_plan(*width, *height, *exif_orientation, *edit)
            .map_or_else(cannot_decode, |plan| PrepareDecision::Run { plan: PreparePlan::Image { plan, quality: super::jpeg_quality(quality.unwrap_or(0.9)), format: *format } }),
        _ => cannot_decode(),
    }
}

/// The width, height and MIME of the file the prepare will make (D-413), or `None` when the descriptor is broken or the probe does not fit it.
#[uniffi::export]
pub fn media_prepare_facts(descriptor: Vec<u8>, probe: PrepareProbe) -> Option<MediaPrepareFacts> {
    let intent = decode_descriptor(&descriptor).ok()?;
    match (&intent, &probe) {
        (MediaPrepareIntent::Image { edit, format, .. }, PrepareProbe::Image { width, height, exif_orientation, .. }) => {
            let size = photo_plan(*width, *height, *exif_orientation, *edit)?.output;
            Some(MediaPrepareFacts { width: size.width, height: size.height, mime: image_mime(*format).to_string() })
        }
        (MediaPrepareIntent::Video { limits, mirror, .. }, PrepareProbe::Video { source, size_bytes, .. }) => {
            let plan = plan_video_sized(*source, *size_bytes, limits.clone().unwrap_or_else(super::default_video_limits)).ok()?;
            let size = if plan.reencode || *mirror { PixelSize { width: plan.width, height: plan.height } } else { upright(source.width, source.height, source.rotation_degrees) };
            Some(MediaPrepareFacts { width: size.width, height: size.height, mime: VIDEO_MIME.to_string() })
        }
        _ => None,
    }
}

/// The check of the written result against `max_bytes` (D-412): `OverMaxBytes` when it is larger, `UnsupportedDescriptor` for a broken descriptor.
#[uniffi::export]
pub fn media_prepare_check_result(descriptor: Vec<u8>, size_bytes: u64) -> Option<MediaPrepareFailure> {
    match decode_descriptor(&descriptor) {
        Err(reason) => Some(reason),
        Ok(MediaPrepareIntent::Video { max_bytes, .. } | MediaPrepareIntent::Image { max_bytes, .. }) => max_bytes.filter(|limit| size_bytes > *limit).map(|_| MediaPrepareFailure::OverMaxBytes),
    }
}

fn image_mime(format: ImageFormat) -> &'static str {
    match format {
        ImageFormat::Jpeg => "image/jpeg",
        ImageFormat::Png => "image/png",
    }
}

fn upright(width: u32, height: u32, rotation_degrees: i32) -> PixelSize {
    if rotation_degrees.rem_euclid(180) == 90 { PixelSize { width: height, height: width } } else { PixelSize { width, height } }
}

fn fraction(value: f64) -> bool {
    (0.0..=1.0).contains(&value)
}

fn side_set(size: Option<PixelSize>) -> bool {
    size.is_none_or(|size| size.width > 0 && size.height > 0)
}

/// The settings the planner cannot work with (D-364): refused with the reason.
fn check_settings(intent: &MediaPrepareIntent) -> Result<(), String> {
    match intent {
        MediaPrepareIntent::Video { limits, .. } => limits.as_ref().map_or(Ok(()), |limits| check(limits).map_err(|error| error.to_string())),
        MediaPrepareIntent::Image { edit, quality, .. } => {
            if quality.is_some_and(|quality| !fraction(quality)) {
                return Err("the image quality is outside 0..1".to_string());
            }
            if !side_set(edit.max_size) || !side_set(edit.output_size) {
                return Err("a size of the image edit has a side of zero".to_string());
            }
            if edit.crop.is_some_and(|crop| !(fraction(crop.x) && fraction(crop.y) && fraction(crop.width) && fraction(crop.height) && crop.width > 0.0 && crop.height > 0.0)) {
                return Err("the crop of the image edit is outside 0..1".to_string());
            }
            if !matches!(edit.rotation_degrees, 0 | 90 | 180 | 270) {
                return Err("the rotation of the image edit is not 0, 90, 180 or 270".to_string());
            }
            Ok(())
        }
    }
}

fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend(value.to_be_bytes());
}

fn put_size(out: &mut Vec<u8>, size: PixelSize) {
    put_u32(out, size.width);
    put_u32(out, size.height);
}

fn put_option<T>(out: &mut Vec<u8>, value: Option<T>, put: impl FnOnce(&mut Vec<u8>, T)) {
    match value {
        None => out.push(0),
        Some(value) => {
            out.push(1);
            put(out, value);
        }
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
}

impl Reader<'_> {
    fn take<const N: usize>(&mut self) -> Option<[u8; N]> {
        let (head, rest) = self.bytes.split_first_chunk::<N>()?;
        self.bytes = rest;
        Some(*head)
    }

    fn u8(&mut self) -> Option<u8> {
        self.take::<1>().map(|[byte]| byte)
    }

    fn bool(&mut self) -> Option<bool> {
        match self.u8()? {
            0 => Some(false),
            1 => Some(true),
            _ => None,
        }
    }

    fn u32(&mut self) -> Option<u32> {
        self.take().map(u32::from_be_bytes)
    }

    fn f64(&mut self) -> Option<f64> {
        self.take().map(f64::from_be_bytes)
    }

    fn size(&mut self) -> Option<PixelSize> {
        Some(PixelSize { width: self.u32()?, height: self.u32()? })
    }

    /// `None` for a broken input, `Some(None)` for an absent value.
    fn option<T>(&mut self, read: impl FnOnce(&mut Self) -> Option<T>) -> Option<Option<T>> {
        if self.bool()? { read(self).map(Some) } else { Some(None) }
    }
}

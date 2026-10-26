use super::profile::{CaptureMode, CaptureProfile};
use crate::ffi::PixelSize;

const HIGH_PHOTO_PIXELS: u64 = 1080 * 1920;
const ASPECT_TOLERANCE: f64 = 0.02;
const ASPECT_MISMATCH_WEIGHT: f64 = 100.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct FpsRange {
    pub low: u32,
    pub high: u32,
}

/// One format a camera offers: the video size, the photo sizes that go with it, the frame rates and the two quality flags of the photo.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FormatCandidate {
    pub video_size: PixelSize,
    pub photo_sizes: Vec<PixelSize>,
    pub frame_rates: Vec<FpsRange>,
    pub high_photo_quality: bool,
    pub highest_photo_quality: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct FormatChoice {
    pub index: u32,
    pub photo_size: Option<PixelSize>,
    pub frame_rate: FpsRange,
}

enum Constraint {
    Fps(u32),
    Video(PixelSize),
    Photo(PixelSize),
    HighPhoto,
    HighestPhoto,
}

fn area(size: &PixelSize) -> u64 {
    u64::from(size.width) * u64::from(size.height)
}

fn ratio(size: &PixelSize) -> f64 {
    f64::from(size.width.max(size.height)) / f64::from(size.width.min(size.height))
}

fn same_aspect(first: &PixelSize, second: &PixelSize) -> bool {
    (ratio(first) - ratio(second)).abs() / ratio(second) < ASPECT_TOLERANCE
}

/// The distance of a size from a target: the aspect ratio counts 100 times its relative gap beyond 2 %, the area counts the log of the ratio of the areas.
fn size_penalty(size: &PixelSize, target: &PixelSize) -> f64 {
    let target_ratio = ratio(target);
    let gap = (ratio(size) - target_ratio).abs() / target_ratio;
    let mismatch = if gap < ASPECT_TOLERANCE { 0.0 } else { ASPECT_MISMATCH_WEIGHT * gap };
    mismatch + (area(size) as f64 / area(target) as f64).ln().abs()
}

fn fps_penalty(ranges: &[FpsRange], target: u32) -> f64 {
    let target = i64::from(target);
    ranges.iter().map(|range| (i64::from(range.low) - target).max(target - i64::from(range.high)).max(0)).min().unwrap_or(0) as f64
}

fn nearest_range(ranges: &[FpsRange], target: u32) -> FpsRange {
    ranges
        .iter()
        .min_by(|first, second| {
            let key = |range: &&FpsRange| (fps_penalty(std::slice::from_ref(*range), target), range.high - range.low);
            let (first_key, second_key) = (key(first), key(second));
            first_key.0.total_cmp(&second_key.0).then(first_key.1.cmp(&second_key.1))
        })
        .copied()
        .unwrap_or(FpsRange { low: target, high: target })
}

fn distance(candidate: &FormatCandidate, constraint: &Constraint) -> f64 {
    match constraint {
        Constraint::Fps(target) => fps_penalty(&candidate.frame_rates, *target),
        Constraint::Video(target) => size_penalty(&candidate.video_size, target),
        Constraint::Photo(target) => candidate.photo_sizes.iter().map(|size| size_penalty(size, target)).min_by(f64::total_cmp).unwrap_or(f64::INFINITY),
        Constraint::HighPhoto => f64::from(u8::from(!candidate.high_photo_quality)),
        Constraint::HighestPhoto => f64::from(u8::from(!candidate.highest_photo_quality)),
    }
}

fn constraints(profile: &CaptureProfile) -> Vec<Constraint> {
    let mut list = vec![Constraint::Fps(profile.fps)];
    list.extend(profile.video_size.map(Constraint::Video));
    if let Some(target) = profile.photo_size {
        list.extend([Constraint::Photo(target), Constraint::HighPhoto, Constraint::HighestPhoto]);
    }
    list
}

/// The formats of a camera that has one list of video sizes, one list of JPEG sizes and one list of frame rates (Android): each JPEG size of the aspect of a video size
/// goes with it, the photo is "high" from 1080x1920 pixels and "highest" when it is the largest JPEG of the camera.
#[uniffi::export]
pub fn format_candidates(video_sizes: Vec<PixelSize>, jpeg_sizes: Vec<PixelSize>, rates: Vec<FpsRange>) -> Vec<FormatCandidate> {
    let largest = jpeg_sizes.iter().map(area).max().unwrap_or(0);
    video_sizes
        .into_iter()
        .map(|video| {
            let photos: Vec<PixelSize> = jpeg_sizes.iter().filter(|size| same_aspect(size, &video)).copied().collect();
            let best = photos.iter().map(area).max().unwrap_or(0);
            FormatCandidate { video_size: video, photo_sizes: photos, frame_rates: rates.clone(), high_photo_quality: best >= HIGH_PHOTO_PIXELS, highest_photo_quality: best > 0 && best == largest }
        })
        .collect()
}

/// The one format for a profile, the same rule on both platforms. The penalty of a format is the sum of the distances to the wishes of the profile
/// (frame rate, video size, photo size, then the two photo quality flags), the first wish weighs the most. A photo profile skips formats without photo sizes.
#[uniffi::export]
pub fn pick_format(candidates: Vec<FormatCandidate>, profile: CaptureProfile) -> Option<FormatChoice> {
    let wishes = constraints(&profile);
    let needs_photo = profile.photo_size.is_some();
    let count = wishes.len();
    let penalty = |candidate: &FormatCandidate| -> f64 { wishes.iter().enumerate().map(|(index, wish)| distance(candidate, wish) * (count - index) as f64).sum() };
    let (index, chosen) = candidates
        .iter()
        .enumerate()
        .filter(|(_, candidate)| !needs_photo || !candidate.photo_sizes.is_empty())
        .min_by(|(_, first), (_, second)| penalty(first).total_cmp(&penalty(second)))?;
    let photo_size = profile.photo_size.and_then(|target| chosen.photo_sizes.iter().min_by(|first, second| size_penalty(first, &target).total_cmp(&size_penalty(second, &target))).copied());
    Some(FormatChoice { index: index as u32, photo_size, frame_rate: nearest_range(&chosen.frame_rates, profile.fps) })
}

impl CaptureProfile {
    pub(crate) fn is_video(&self) -> bool {
        self.mode == CaptureMode::Video
    }
}

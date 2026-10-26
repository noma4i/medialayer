use super::geometry::{CropAspect, CropSetup};
use super::view::{crop_layout, CropView};
use crate::send_prep::CropFractions;

/// The kind of one finished gesture: `Pinch` when the fingers scaled at any time during it, otherwise `Pan`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum CropGesture {
    Pan,
    Pinch,
}

/// The moves of one gesture so far, from `crop_gesture_begin` through `crop_gesture_move`. `max_translate` is the largest move of the photo
/// (of the frame when it is free) from the frame at the start in view points, `max_scale_change` the largest change of the raw scale from the start.
#[derive(Debug, Clone, Copy, PartialEq, uniffi::Record)]
pub struct CropGestureTrack {
    pub start: CropFractions,
    pub max_translate: f64,
    pub max_scale_change: f64,
}

/// What one finished gesture reports to the app, in the units of the V2 and V3 analytics event.
/// `max_distance` is the largest move in view points for a pan, the largest scale change for a pinch, both rounded to a whole number.
/// `scale_change` is the scale at the end minus the scale at the start (rounded to 0.01), `translate_change` the length of the move in view points between the start and the end.
/// `final_scale`, `final_translate_x` and `final_translate_y` are the transform against the first frame after the gesture, as in `CropTelemetry`.
#[derive(Debug, Clone, Copy, PartialEq, uniffi::Record)]
pub struct CropGestureReport {
    pub kind: CropGesture,
    pub duration_ms: u64,
    pub max_distance: i32,
    pub scale_change: f64,
    pub translate_change: i32,
    pub final_scale: f64,
    pub final_translate_x: i32,
    pub final_translate_y: i32,
}

/// What a finished gesture tells the app: `report` is `None` when the gesture moved and scaled nothing.
/// `was_position_changed` stays true once any gesture moved or scaled the crop.
#[derive(Debug, Clone, Copy, PartialEq, uniffi::Record)]
pub struct CropGestureEnd {
    pub report: Option<CropGestureReport>,
    pub was_position_changed: bool,
}

/// The final transform of the crop against its first frame. `final_scale` is 1.0 for the first frame, above 1 when the user magnified, rounded to 0.01.
/// `final_translate_x` and `final_translate_y` are the move of the photo (the frame when it is free) in whole view points or pixels, right and down are positive.
#[derive(Debug, Clone, Copy, PartialEq, uniffi::Record)]
pub struct CropTelemetry {
    pub final_scale: f64,
    pub final_translate_x: i32,
    pub final_translate_y: i32,
    pub was_position_changed: bool,
}

#[uniffi::export]
pub fn crop_gesture_begin(rect: CropFractions) -> CropGestureTrack {
    CropGestureTrack { start: rect, max_translate: 0.0, max_scale_change: 0.0 }
}

/// Call after every step of the gesture with the frame now. `baseline` is the first frame of the crop (turned with the photo).
#[uniffi::export]
pub fn crop_gesture_move(track: CropGestureTrack, baseline: CropFractions, rect: CropFractions, setup: CropSetup, view: CropView) -> CropGestureTrack {
    let (translate, scale_change) = since_start(track.start, baseline, rect, setup, view);
    CropGestureTrack { start: track.start, max_translate: track.max_translate.max(translate), max_scale_change: track.max_scale_change.max(scale_change.abs()) }
}

/// `started_ms` and `ended_ms` are readings of one monotonic clock of the platform at the start and at the end of the gesture.
#[uniffi::export]
#[allow(clippy::too_many_arguments)]
pub fn crop_gesture_end(
    track: CropGestureTrack,
    baseline: CropFractions,
    rect: CropFractions,
    setup: CropSetup,
    view: CropView,
    panned: bool,
    zoomed: bool,
    was_position_changed: bool,
    started_ms: u64,
    ended_ms: u64,
) -> CropGestureEnd {
    let kind = match (panned, zoomed) {
        (_, true) => Some(CropGesture::Pinch),
        (true, false) => Some(CropGesture::Pan),
        (false, false) => None,
    };
    let report = kind.map(|kind| {
        let (translate, scale_change) = since_start(track.start, baseline, rect, setup, view);
        let telemetry = crop_telemetry(baseline, rect, setup, view, true);
        let largest = if kind == CropGesture::Pinch { track.max_scale_change } else { track.max_translate };
        CropGestureReport {
            kind,
            duration_ms: ended_ms.saturating_sub(started_ms),
            max_distance: largest.round() as i32,
            scale_change: round_hundredths(scale_change),
            translate_change: translate.round() as i32,
            final_scale: telemetry.final_scale,
            final_translate_x: telemetry.final_translate_x,
            final_translate_y: telemetry.final_translate_y,
        }
    });
    CropGestureEnd { report, was_position_changed: was_position_changed || kind.is_some() }
}

fn round_hundredths(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

fn raw_scale(baseline: CropFractions, rect: CropFractions) -> f64 {
    let ratio = (baseline.width * baseline.height / (rect.width * rect.height)).sqrt();
    if ratio.is_finite() && ratio > 0.0 {
        ratio
    } else {
        1.0
    }
}

/// The length of the move and the change of the raw scale between the frame at the start of a gesture and the frame now.
fn since_start(start: CropFractions, baseline: CropFractions, rect: CropFractions, setup: CropSetup, view: CropView) -> (f64, f64) {
    let translate = match (centre(start, setup, view), centre(rect, setup, view)) {
        (Some(from), Some(to)) => (to.0 - from.0).hypot(to.1 - from.1),
        _ => 0.0,
    };
    (translate, raw_scale(baseline, rect) - raw_scale(baseline, start))
}

fn centre(rect: CropFractions, setup: CropSetup, view: CropView) -> Option<(f64, f64)> {
    let layout = crop_layout(rect, setup, view)?;
    let moving = if setup.aspect.is_some_and(|shape: CropAspect| shape.width > 0 && shape.height > 0) { layout.photo } else { layout.frame };
    Some((moving.x + moving.width / 2.0, moving.y + moving.height / 2.0))
}

/// `baseline` is the first frame of the crop (turned with the photo), `rect` the frame now. A frame or a view that cannot be laid out gives scale 1 and no move.
#[uniffi::export]
pub fn crop_telemetry(baseline: CropFractions, rect: CropFractions, setup: CropSetup, view: CropView, was_position_changed: bool) -> CropTelemetry {
    let final_scale = round_hundredths(raw_scale(baseline, rect));
    let (final_translate_x, final_translate_y) = match (centre(baseline, setup, view), centre(rect, setup, view)) {
        (Some(from), Some(to)) => ((to.0 - from.0).round() as i32, (to.1 - from.1).round() as i32),
        _ => (0, 0),
    };
    CropTelemetry { final_scale, final_translate_x, final_translate_y, was_position_changed }
}

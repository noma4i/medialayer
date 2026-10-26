use super::geometry::{CropAspect, CropSetup};
use super::view::{crop_layout, CropView};
use crate::send_prep::CropFractions;

/// The kind of one finished gesture: `Pinch` when the fingers scaled at any time during it, otherwise `Pan`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum CropGesture {
    Pan,
    Pinch,
}

/// What a finished gesture tells the app: `kind` is `None` when the gesture moved and scaled nothing.
/// `was_position_changed` stays true once any gesture moved or scaled the crop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct CropGestureEnd {
    pub kind: Option<CropGesture>,
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
pub fn crop_gesture_end(panned: bool, zoomed: bool, was_position_changed: bool) -> CropGestureEnd {
    let kind = match (panned, zoomed) {
        (_, true) => Some(CropGesture::Pinch),
        (true, false) => Some(CropGesture::Pan),
        (false, false) => None,
    };
    CropGestureEnd { kind, was_position_changed: was_position_changed || kind.is_some() }
}

fn centre(rect: CropFractions, setup: CropSetup, view: CropView) -> Option<(f64, f64)> {
    let layout = crop_layout(rect, setup, view)?;
    let moving = if setup.aspect.is_some_and(|shape: CropAspect| shape.width > 0 && shape.height > 0) { layout.photo } else { layout.frame };
    Some((moving.x + moving.width / 2.0, moving.y + moving.height / 2.0))
}

/// `baseline` is the first frame of the crop (turned with the photo), `rect` the frame now. A frame or a view that cannot be laid out gives scale 1 and no move.
#[uniffi::export]
pub fn crop_telemetry(baseline: CropFractions, rect: CropFractions, setup: CropSetup, view: CropView, was_position_changed: bool) -> CropTelemetry {
    let area = |frame: CropFractions| frame.width * frame.height;
    let ratio = (area(baseline) / area(rect)).sqrt();
    let final_scale = if ratio.is_finite() && ratio > 0.0 { (ratio * 100.0).round() / 100.0 } else { 1.0 };
    let (final_translate_x, final_translate_y) = match (centre(baseline, setup, view), centre(rect, setup, view)) {
        (Some(from), Some(to)) => ((to.0 - from.0).round() as i32, (to.1 - from.1).round() as i32),
        _ => (0, 0),
    };
    CropTelemetry { final_scale, final_translate_x, final_translate_y, was_position_changed }
}

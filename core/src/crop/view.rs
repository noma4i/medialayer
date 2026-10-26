use super::geometry::{crop_drag, crop_pan, crop_zoom, CropAspect, CropHandle, CropSetup};
use crate::send_prep::CropFractions;

/// The area the crop is drawn in, in view pixels or points. `inset` is the margin kept free on every side.
#[derive(Debug, Clone, Copy, PartialEq, uniffi::Record)]
pub struct CropView {
    pub width: f64,
    pub height: f64,
    pub inset: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, uniffi::Record)]
pub struct CropBox {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Where to draw: `photo` is the whole photo, `frame` is the crop frame. A crop with proportions keeps `frame` still and moves the photo under it,
/// a free crop keeps `photo` still and moves `frame`.
#[derive(Debug, Clone, Copy, PartialEq, uniffi::Record)]
pub struct CropLayout {
    pub photo: CropBox,
    pub frame: CropBox,
}

fn proportions(aspect: Option<CropAspect>) -> Option<CropAspect> {
    aspect.filter(|shape| shape.width > 0 && shape.height > 0)
}

#[uniffi::export]
pub fn crop_layout(rect: CropFractions, setup: CropSetup, view: CropView) -> Option<CropLayout> {
    let CropSetup { photo, aspect, .. } = setup;
    let inner_width = view.width - 2.0 * view.inset;
    let inner_height = view.height - 2.0 * view.inset;
    let sound = [rect.x, rect.y, rect.width, rect.height, view.width, view.height, view.inset].iter().all(|value| value.is_finite());
    if !sound || photo.width == 0 || photo.height == 0 || inner_width <= 0.0 || inner_height <= 0.0 || rect.width <= 0.0 || rect.height <= 0.0 {
        return None;
    }
    let (width, height) = (f64::from(photo.width), f64::from(photo.height));
    match proportions(aspect) {
        None => {
            let scale = (inner_width / width).min(inner_height / height);
            let (photo_width, photo_height) = (width * scale, height * scale);
            let (photo_x, photo_y) = ((view.width - photo_width) / 2.0, (view.height - photo_height) / 2.0);
            Some(CropLayout {
                photo: CropBox { x: photo_x, y: photo_y, width: photo_width, height: photo_height },
                frame: CropBox { x: photo_x + rect.x * photo_width, y: photo_y + rect.y * photo_height, width: rect.width * photo_width, height: rect.height * photo_height },
            })
        }
        Some(shape) => {
            let wide = f64::from(shape.width) / f64::from(shape.height);
            let frame_width = inner_width.min(inner_height * wide);
            let frame_height = frame_width / wide;
            let (frame_x, frame_y) = ((view.width - frame_width) / 2.0, (view.height - frame_height) / 2.0);
            let (photo_width, photo_height) = (frame_width / rect.width, frame_height / rect.height);
            Some(CropLayout {
                photo: CropBox { x: frame_x - rect.x * photo_width, y: frame_y - rect.y * photo_height, width: photo_width, height: photo_height },
                frame: CropBox { x: frame_x, y: frame_y, width: frame_width, height: frame_height },
            })
        }
    }
}

/// The rectangle after the finger moved by `dx` and `dy` view pixels. With proportions the photo follows the finger, a free frame follows it as well.
#[uniffi::export]
pub fn crop_pan_view(rect: CropFractions, setup: CropSetup, view: CropView, dx: f64, dy: f64) -> CropFractions {
    let Some(layout) = crop_layout(rect, setup, view) else { return rect };
    let sign = if proportions(setup.aspect).is_some() { -1.0 } else { 1.0 };
    crop_pan(rect, sign * dx / layout.photo.width, sign * dy / layout.photo.height)
}

/// The rectangle after a pinch of `factor` (above 1 magnifies) around the view point (`focal_x`, `focal_y`).
#[uniffi::export]
pub fn crop_zoom_view(rect: CropFractions, setup: CropSetup, view: CropView, factor: f64, focal_x: f64, focal_y: f64) -> CropFractions {
    let Some(layout) = crop_layout(rect, setup, view) else { return rect };
    crop_zoom(rect, setup, factor, (focal_x - layout.photo.x) / layout.photo.width, (focal_y - layout.photo.y) / layout.photo.height)
}

/// The rectangle after a handle moved by `dx` and `dy` view pixels.
#[uniffi::export]
pub fn crop_drag_view(rect: CropFractions, setup: CropSetup, view: CropView, handle: CropHandle, dx: f64, dy: f64) -> CropFractions {
    let Some(layout) = crop_layout(rect, setup, view) else { return rect };
    crop_drag(rect, setup, handle, dx / layout.photo.width, dy / layout.photo.height)
}

/// The handle under the view point (`x`, `y`), a corner when the point is within `slop` of two edges, a side within `slop` of one edge. `None` for the inside and for the outside.
#[uniffi::export]
pub fn crop_handle_at(frame: CropBox, x: f64, y: f64, slop: f64) -> Option<CropHandle> {
    if ![frame.x, frame.y, frame.width, frame.height, x, y, slop].iter().all(|value| value.is_finite()) {
        return None;
    }
    let inside_x = x >= frame.x - slop && x <= frame.x + frame.width + slop;
    let inside_y = y >= frame.y - slop && y <= frame.y + frame.height + slop;
    if !inside_x || !inside_y {
        return None;
    }
    let left = (x - frame.x).abs() <= slop;
    let right = (x - frame.x - frame.width).abs() <= slop;
    let top = (y - frame.y).abs() <= slop;
    let bottom = (y - frame.y - frame.height).abs() <= slop;
    match (left, right, top, bottom) {
        (true, _, true, _) => Some(CropHandle::TopLeft),
        (_, true, true, _) => Some(CropHandle::TopRight),
        (true, _, _, true) => Some(CropHandle::BottomLeft),
        (_, true, _, true) => Some(CropHandle::BottomRight),
        (true, ..) => Some(CropHandle::Left),
        (_, true, ..) => Some(CropHandle::Right),
        (_, _, true, _) => Some(CropHandle::Top),
        (_, _, _, true) => Some(CropHandle::Bottom),
        _ => None,
    }
}

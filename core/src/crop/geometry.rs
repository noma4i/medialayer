use crate::ffi::PixelSize;
use crate::send_prep::CropFractions;

const EPSILON: f64 = 1e-9;
const DEFAULT_MIN_SIDE: f64 = 0.1;
const DEFAULT_MAX_ZOOM: f64 = 3.0;

/// The proportions of a fixed crop frame, width to height. A side of zero means no proportion: the crop is free.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct CropAspect {
    pub width: u32,
    pub height: u32,
}

/// `min_side` is the smallest side of a free crop as a fraction of the shorter side of the photo.
/// `max_zoom` is how many times a fixed crop may shrink from its largest size.
/// A value that is not a number, or out of range, gives the default.
#[derive(Debug, Clone, Copy, PartialEq, uniffi::Record)]
pub struct CropLimits {
    pub min_side: f64,
    pub max_zoom: f64,
}

/// The photo size (after its turns), the proportions and the limits of one crop. Every function of the crop reads the same setup.
#[derive(Debug, Clone, Copy, PartialEq, uniffi::Record)]
pub struct CropSetup {
    pub photo: PixelSize,
    pub aspect: Option<CropAspect>,
    pub limits: CropLimits,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum CropHandle {
    Left,
    Top,
    Right,
    Bottom,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

#[uniffi::export]
pub fn default_crop_limits() -> CropLimits {
    CropLimits { min_side: DEFAULT_MIN_SIDE, max_zoom: DEFAULT_MAX_ZOOM }
}

struct Bounds {
    min_w: f64,
    min_h: f64,
    max_w: f64,
    max_h: f64,
    /// Height per width in fractions of the photo, `None` for a free crop.
    ratio: Option<f64>,
}

fn bounds(photo: PixelSize, aspect: Option<CropAspect>, limits: CropLimits) -> Option<Bounds> {
    if photo.width == 0 || photo.height == 0 {
        return None;
    }
    let (width, height) = (f64::from(photo.width), f64::from(photo.height));
    let min_side = if limits.min_side.is_finite() && limits.min_side > 0.0 { limits.min_side.min(1.0) } else { DEFAULT_MIN_SIDE };
    let max_zoom = if limits.max_zoom.is_finite() && limits.max_zoom >= 1.0 { limits.max_zoom } else { DEFAULT_MAX_ZOOM };
    match aspect.filter(|shape| shape.width > 0 && shape.height > 0) {
        None => {
            let shorter = width.min(height);
            Some(Bounds { min_w: min_side * shorter / width, min_h: min_side * shorter / height, max_w: 1.0, max_h: 1.0, ratio: None })
        }
        Some(shape) => {
            let ratio = width * f64::from(shape.height) / (f64::from(shape.width) * height);
            let max_w = 1.0_f64.min(1.0 / ratio);
            Some(Bounds { min_w: max_w / max_zoom, min_h: max_w / max_zoom * ratio, max_w, max_h: max_w * ratio, ratio: Some(ratio) })
        }
    }
}

fn full() -> CropFractions {
    CropFractions { x: 0.0, y: 0.0, width: 1.0, height: 1.0 }
}

fn finite(rect: CropFractions) -> bool {
    [rect.x, rect.y, rect.width, rect.height].iter().all(|value| value.is_finite()) && rect.width > 0.0 && rect.height > 0.0
}

fn limit(value: f64, low: f64, high: f64) -> f64 {
    value.min(high).max(low)
}

fn placed(center_x: f64, center_y: f64, width: f64, height: f64) -> CropFractions {
    CropFractions { x: limit(center_x - width / 2.0, 0.0, 1.0 - width), y: limit(center_y - height / 2.0, 0.0, 1.0 - height), width, height }
}

/// The first rectangle of a crop. Without `initial`: the largest frame of the proportions in the middle of the photo, the whole photo when free.
/// With `initial`: the frame is fitted to the proportions and the limits and keeps the middle of `initial`.
#[uniffi::export]
pub fn crop_initial(setup: CropSetup, initial: Option<CropFractions>) -> CropFractions {
    let Some(bounds) = bounds(setup.photo, setup.aspect, setup.limits) else { return full() };
    let start = initial.filter(|rect| finite(*rect));
    let (center_x, center_y) = start.map_or((0.5, 0.5), |rect| (rect.x + rect.width / 2.0, rect.y + rect.height / 2.0));
    match bounds.ratio {
        None => match start {
            None => full(),
            Some(rect) => placed(center_x, center_y, limit(rect.width, bounds.min_w, bounds.max_w), limit(rect.height, bounds.min_h, bounds.max_h)),
        },
        Some(ratio) => {
            let width = start.map_or(bounds.max_w, |rect| rect.width.min(rect.height / ratio));
            let width = limit(width, bounds.min_w, bounds.max_w);
            placed(center_x, center_y, width, width * ratio)
        }
    }
}

/// The rectangle moved by `dx` and `dy` (fractions of the photo). The size stays and the edges stay inside the photo.
#[uniffi::export]
pub fn crop_pan(rect: CropFractions, dx: f64, dy: f64) -> CropFractions {
    if !finite(rect) {
        return rect;
    }
    let (dx, dy) = (if dx.is_finite() { dx } else { 0.0 }, if dy.is_finite() { dy } else { 0.0 });
    CropFractions { x: limit(rect.x + dx, 0.0, 1.0 - rect.width), y: limit(rect.y + dy, 0.0, 1.0 - rect.height), ..rect }
}

/// The rectangle resized by `factor` (above 1 makes it smaller, the content looks larger) around the photo point (`focal_x`, `focal_y`).
/// The point keeps its place inside the rectangle. The size follows the limits and the proportions.
#[uniffi::export]
pub fn crop_zoom(rect: CropFractions, setup: CropSetup, factor: f64, focal_x: f64, focal_y: f64) -> CropFractions {
    let Some(bounds) = bounds(setup.photo, setup.aspect, setup.limits) else { return rect };
    if !finite(rect) || !factor.is_finite() || factor <= 0.0 {
        return rect;
    }
    let share = |point: f64, start: f64, length: f64| if point.is_finite() { ((point - start) / length).clamp(0.0, 1.0) } else { 0.5 };
    let (across, down) = (share(focal_x, rect.x, rect.width), share(focal_y, rect.y, rect.height));
    let (width, height) = match bounds.ratio {
        None => {
            let most = (rect.width / bounds.min_w).min(rect.height / bounds.min_h);
            let least = (rect.width / bounds.max_w).max(rect.height / bounds.max_h);
            let scale = factor.min(most).max(least);
            (rect.width / scale, rect.height / scale)
        }
        Some(ratio) => {
            let width = limit(rect.width / factor, bounds.min_w, bounds.max_w);
            (width, width * ratio)
        }
    };
    CropFractions {
        x: limit(rect.x + across * rect.width - across * width, 0.0, 1.0 - width),
        y: limit(rect.y + down * rect.height - down * height, 0.0, 1.0 - height),
        width,
        height,
    }
}

/// The rectangle with one handle moved by `dx` and `dy`. The opposite edge or corner stays. A free crop moves each edge alone.
/// A crop with proportions answers only corners and keeps its proportions, a side handle leaves the rectangle as it is.
#[uniffi::export]
pub fn crop_drag(rect: CropFractions, setup: CropSetup, handle: CropHandle, dx: f64, dy: f64) -> CropFractions {
    let Some(bounds) = bounds(setup.photo, setup.aspect, setup.limits) else { return rect };
    if !finite(rect) || !dx.is_finite() || !dy.is_finite() {
        return rect;
    }
    let left = matches!(handle, CropHandle::Left | CropHandle::TopLeft | CropHandle::BottomLeft);
    let right = matches!(handle, CropHandle::Right | CropHandle::TopRight | CropHandle::BottomRight);
    let top = matches!(handle, CropHandle::Top | CropHandle::TopLeft | CropHandle::TopRight);
    let bottom = matches!(handle, CropHandle::Bottom | CropHandle::BottomLeft | CropHandle::BottomRight);
    let (mut first_x, mut first_y, mut last_x, mut last_y) = (rect.x, rect.y, rect.x + rect.width, rect.y + rect.height);
    match bounds.ratio {
        None => {
            if left {
                first_x = limit(first_x + dx, 0.0, last_x - bounds.min_w);
            }
            if right {
                last_x = limit(last_x + dx, first_x + bounds.min_w, 1.0);
            }
            if top {
                first_y = limit(first_y + dy, 0.0, last_y - bounds.min_h);
            }
            if bottom {
                last_y = limit(last_y + dy, first_y + bounds.min_h, 1.0);
            }
            CropFractions { x: first_x, y: first_y, width: last_x - first_x, height: last_y - first_y }
        }
        Some(ratio) => {
            if !(left || right) || !(top || bottom) {
                return rect;
            }
            let (anchor_x, anchor_y) = (if right { first_x } else { last_x }, if bottom { first_y } else { last_y });
            let by_width = rect.width + if right { dx } else { -dx };
            let by_height = (rect.height + if bottom { dy } else { -dy }) / ratio;
            let room = (if right { 1.0 - anchor_x } else { anchor_x }).min((if bottom { 1.0 - anchor_y } else { anchor_y }) / ratio).min(bounds.max_w);
            let width = limit((by_width + by_height) / 2.0, bounds.min_w, room.max(bounds.min_w));
            let height = width * ratio;
            CropFractions {
                x: limit(if right { anchor_x } else { anchor_x - width }, 0.0, 1.0 - width),
                y: limit(if bottom { anchor_y } else { anchor_y - height }, 0.0, 1.0 - height),
                width,
                height,
            }
        }
    }
}

/// The rectangle after the photo turns a quarter clockwise or counter clockwise. `setup.photo` is the size before the turn.
/// A crop with proportions is fitted again around the middle of the turned rectangle.
#[uniffi::export]
pub fn crop_turn(rect: CropFractions, setup: CropSetup, clockwise: bool) -> CropFractions {
    let turned_setup = CropSetup { photo: PixelSize { width: setup.photo.height, height: setup.photo.width }, ..setup };
    if !finite(rect) {
        return crop_initial(turned_setup, None);
    }
    let turned = if clockwise {
        CropFractions { x: 1.0 - rect.y - rect.height, y: rect.x, width: rect.height, height: rect.width }
    } else {
        CropFractions { x: rect.y, y: 1.0 - rect.x - rect.width, width: rect.height, height: rect.width }
    };
    crop_initial(turned_setup, Some(turned))
}

/// The rectangle the app may use: it keeps its edges inside the photo, its size inside the limits, and with proportions its pixels stay within one pixel per edge of them.
/// `None` for a number that is not finite, a rectangle that is too large or too small, or proportions out of tolerance.
#[uniffi::export]
pub fn crop_finish(rect: CropFractions, setup: CropSetup) -> Option<CropFractions> {
    let CropSetup { photo, aspect, limits } = setup;
    let bounds = bounds(photo, aspect, limits)?;
    if !finite(rect) {
        return None;
    }
    let (too_large, too_small) = (rect.width > bounds.max_w + EPSILON || rect.height > bounds.max_h + EPSILON, rect.width < bounds.min_w - EPSILON || rect.height < bounds.min_h - EPSILON);
    if too_large || too_small {
        return None;
    }
    let kept = CropFractions { x: limit(rect.x, 0.0, 1.0 - rect.width), y: limit(rect.y, 0.0, 1.0 - rect.height), ..rect };
    if let Some(shape) = aspect.filter(|shape| shape.width > 0 && shape.height > 0) {
        let edge = |fraction: f64, total: u32| (fraction.clamp(0.0, 1.0) * f64::from(total)).round();
        let pixels_wide = edge(kept.x + kept.width, photo.width) - edge(kept.x, photo.width);
        let pixels_high = edge(kept.y + kept.height, photo.height) - edge(kept.y, photo.height);
        let (wide, high) = (f64::from(shape.width), f64::from(shape.height));
        if pixels_wide < 1.0 || pixels_high < 1.0 || (pixels_wide * high - pixels_high * wide).abs() > wide + high {
            return None;
        }
    }
    Some(kept)
}

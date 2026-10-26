use super::*;
use crate::ffi::PixelSize;
use crate::send_prep::{photo_plan, CropFractions, PhotoEdit};

fn photo(width: u32, height: u32) -> PixelSize {
    PixelSize { width, height }
}

fn rect(x: f64, y: f64, width: f64, height: f64) -> CropFractions {
    CropFractions { x, y, width, height }
}

fn chat() -> Option<CropAspect> {
    Some(CropAspect { width: 1080, height: 1920 })
}

fn square() -> Option<CropAspect> {
    Some(CropAspect { width: 1, height: 1 })
}

fn near(actual: CropFractions, expected: CropFractions, name: &str) {
    for (got, want) in [(actual.x, expected.x), (actual.y, expected.y), (actual.width, expected.width), (actual.height, expected.height)] {
        assert!((got - want).abs() < 1e-9, "{name}: {actual:?} != {expected:?}");
    }
}

fn inside(rect: CropFractions) -> bool {
    rect.x >= -1e-12 && rect.y >= -1e-12 && rect.x + rect.width <= 1.0 + 1e-12 && rect.y + rect.height <= 1.0 + 1e-12
}

fn pixels_wide_per_high(rect: CropFractions, photo: PixelSize) -> f64 {
    rect.width * f64::from(photo.width) / (rect.height * f64::from(photo.height))
}

fn setup(photo: PixelSize, aspect: Option<CropAspect>) -> CropSetup {
    CropSetup { photo, aspect, limits: default_crop_limits() }
}

fn view() -> CropView {
    CropView { width: 400.0, height: 800.0, inset: 0.0 }
}

#[test]
fn inv_crop_01_the_first_frame_is_the_largest_frame_of_the_proportions_in_the_middle_and_a_free_crop_is_the_whole_photo() {
    for (name, size, aspect, expected) in [
        ("chat on a landscape photo", photo(4000, 3000), chat(), rect(0.5 - 0.5 * 3000.0 * 1080.0 / 1920.0 / 4000.0, 0.0, 3000.0 * 1080.0 / 1920.0 / 4000.0, 1.0)),
        ("chat on a tall photo of the same proportions", photo(1080, 1920), chat(), rect(0.0, 0.0, 1.0, 1.0)),
        ("chat on a wide tall photo", photo(2000, 2000), chat(), rect(0.5 - 0.5 * 1080.0 / 1920.0, 0.0, 1080.0 / 1920.0, 1.0)),
        ("square on a landscape photo", photo(4000, 3000), square(), rect(0.5 - 0.5 * 0.75, 0.0, 0.75, 1.0)),
        ("square on a portrait photo", photo(3000, 4000), square(), rect(0.0, 0.5 - 0.5 * 0.75, 1.0, 0.75)),
        ("free", photo(4000, 3000), None, rect(0.0, 0.0, 1.0, 1.0)),
    ] {
        let first = crop_initial(setup(size, aspect), None);
        near(first, expected, name);
        if aspect.is_some() {
            let shape = aspect.unwrap();
            assert!((pixels_wide_per_high(first, size) - f64::from(shape.width) / f64::from(shape.height)).abs() < 1e-9, "{name}");
        }
    }
    near(crop_initial(setup(photo(0, 100), chat()), None), rect(0.0, 0.0, 1.0, 1.0), "no photo");
}

#[test]
fn inv_crop_02_a_given_start_is_fitted_to_the_proportions_and_the_limits_around_its_middle() {
    let size = photo(2000, 2000);
    for (name, aspect, start, expected) in [
        ("free start kept", None, rect(0.2, 0.3, 0.4, 0.4), rect(0.2, 0.3, 0.4, 0.4)),
        ("free start too small", None, rect(0.45, 0.45, 0.01, 0.01), rect(0.405, 0.405, 0.1, 0.1)),
        ("free start past the edge", None, rect(0.8, 0.8, 0.5, 0.5), rect(0.5, 0.5, 0.5, 0.5)),
        ("square from a wide start", square(), rect(0.1, 0.1, 0.6, 0.2), rect(0.2333333333333333, 0.0333333333333333, 1.0 / 3.0, 1.0 / 3.0)),
        ("square from a tall start", square(), rect(0.1, 0.1, 0.2, 0.6), rect(0.0333333333333333, 0.2333333333333333, 1.0 / 3.0, 1.0 / 3.0)),
    ] {
        let first = crop_initial(setup(size, aspect), Some(start));
        assert!(inside(first), "{name}: {first:?}");
        near(first, expected, name);
    }
    let chat_first = crop_initial(setup(size, chat()), Some(rect(0.25, 0.25, 0.5, 0.5)));
    assert!((chat_first.width / chat_first.height - 1080.0 / 1920.0).abs() < 1e-9);
    assert!((chat_first.x + chat_first.width / 2.0 - 0.5).abs() < 1e-9 && (chat_first.y + chat_first.height / 2.0 - 0.5).abs() < 1e-9, "the middle stays");
    near(crop_initial(setup(size, square()), Some(rect(f64::NAN, 0.0, 0.5, 0.5))), rect(0.0, 0.0, 1.0, 1.0), "not a number is no start");
}

#[test]
fn inv_crop_03_a_pan_keeps_the_size_and_the_edges_inside_the_photo() {
    let start = rect(0.2, 0.3, 0.4, 0.5);
    for (name, dx, dy, expected) in [
        ("inside", 0.1, -0.1, rect(0.3, 0.2, 0.4, 0.5)),
        ("past the right and bottom", 5.0, 5.0, rect(0.6, 0.5, 0.4, 0.5)),
        ("past the left and top", -5.0, -5.0, rect(0.0, 0.0, 0.4, 0.5)),
        ("not a number", f64::NAN, f64::INFINITY, start),
    ] {
        let moved = crop_pan(start, dx, dy);
        near(moved, expected, name);
        assert!(inside(moved) && (moved.width, moved.height) == (start.width, start.height), "{name}");
    }
}

#[test]
fn inv_crop_04_a_zoom_keeps_the_focal_point_in_place_inside_the_limits_and_the_proportions() {
    let size = photo(2000, 3000);
    let first = crop_initial(setup(size, chat()), None);
    let zoomed = crop_zoom(first, setup(size, chat()), 2.0, 0.5, 0.5);
    assert!((zoomed.width - first.width / 2.0).abs() < 1e-9);
    assert!((pixels_wide_per_high(zoomed, size) - 1080.0 / 1920.0).abs() < 1e-9);
    assert!((zoomed.x + zoomed.width / 2.0 - (first.x + first.width / 2.0)).abs() < 1e-9);
    let corner = crop_zoom(first, setup(size, chat()), 2.0, first.x, first.y);
    assert!((corner.x - first.x).abs() < 1e-9 && (corner.y - first.y).abs() < 1e-9, "the top left point stays");
    let most = crop_zoom(first, setup(size, chat()), 100.0, 0.5, 0.5);
    assert!((most.width - first.width / 3.0).abs() < 1e-9, "the largest zoom is 3 times");
    near(crop_zoom(zoomed, setup(size, chat()), 0.01, 0.5, 0.5), first, "zoom out stops at the largest frame");
    let moved = crop_zoom(rect(0.0, 0.0, first.width, first.height), setup(size, chat()), 2.0, 0.25 * first.width, 0.25 * first.height);
    assert!(inside(moved));
    let free = crop_zoom(rect(0.0, 0.0, 1.0, 1.0), setup(size, None), 4.0, 0.5, 0.5);
    near(free, rect(0.375, 0.375, 0.25, 0.25), "free zoom shrinks both sides");
    let floor = crop_zoom(free, setup(size, None), 100.0, 0.5, 0.5);
    assert!((floor.width * 2000.0 - 0.1 * 2000.0).abs() < 1e-6, "the smallest free side is 10 percent of the shorter side: {floor:?}");
    near(crop_zoom(first, setup(size, chat()), f64::NAN, 0.5, 0.5), first, "not a number");
}

#[test]
fn inv_crop_05_a_free_drag_keeps_the_opposite_edge_and_the_smallest_side() {
    let size = photo(2000, 1000);
    let start = rect(0.25, 0.25, 0.5, 0.5);
    for (name, handle, dx, dy, expected) in [
        ("left", CropHandle::Left, -0.1, 0.3, rect(0.15, 0.25, 0.6, 0.5)),
        ("right", CropHandle::Right, 0.1, 0.3, rect(0.25, 0.25, 0.6, 0.5)),
        ("top", CropHandle::Top, 0.3, -0.1, rect(0.25, 0.15, 0.5, 0.6)),
        ("bottom", CropHandle::Bottom, 0.3, 0.1, rect(0.25, 0.25, 0.5, 0.6)),
        ("top left", CropHandle::TopLeft, -0.1, -0.1, rect(0.15, 0.15, 0.6, 0.6)),
        ("bottom right past the photo", CropHandle::BottomRight, 5.0, 5.0, rect(0.25, 0.25, 0.75, 0.75)),
        ("left past the photo", CropHandle::Left, -5.0, 0.0, rect(0.0, 0.25, 0.75, 0.5)),
        ("right past the smallest side", CropHandle::Right, -5.0, 0.0, rect(0.25, 0.25, 0.05, 0.5)),
        ("top past the smallest side", CropHandle::Top, 0.0, 5.0, rect(0.25, 0.65, 0.5, 0.1)),
    ] {
        let moved = crop_drag(start, setup(size, None), handle, dx, dy);
        near(moved, expected, name);
        assert!(inside(moved), "{name}");
    }
    near(crop_drag(start, setup(size, None), CropHandle::Left, f64::NAN, 0.0), start, "not a number");
}

#[test]
fn inv_crop_06_a_drag_with_proportions_moves_corners_only_and_keeps_the_proportions_and_the_anchor() {
    let size = photo(2000, 3000);
    let start = rect(0.25, 0.25, 0.4, 0.4 * 2000.0 / 3000.0 * 1920.0 / 1080.0);
    let shape = chat();
    for handle in [CropHandle::Left, CropHandle::Right, CropHandle::Top, CropHandle::Bottom] {
        near(crop_drag(start, setup(size, shape), handle, 0.1, 0.1), start, "a side refuses");
    }
    for (name, handle, dx, dy) in [
        ("bottom right out", CropHandle::BottomRight, 0.05, 0.02),
        ("bottom right in", CropHandle::BottomRight, -0.05, -0.02),
        ("top left out", CropHandle::TopLeft, -0.03, -0.03),
        ("top right", CropHandle::TopRight, 0.02, -0.04),
        ("bottom left far out", CropHandle::BottomLeft, -9.0, 9.0),
    ] {
        let moved = crop_drag(start, setup(size, shape), handle, dx, dy);
        assert!(inside(moved), "{name}: {moved:?}");
        assert!((pixels_wide_per_high(moved, size) - 1080.0 / 1920.0).abs() < 1e-9, "{name}");
        let (anchor_x, anchor_y) = match handle {
            CropHandle::BottomRight => (moved.x - start.x, moved.y - start.y),
            CropHandle::TopLeft => ((moved.x + moved.width) - (start.x + start.width), (moved.y + moved.height) - (start.y + start.height)),
            CropHandle::TopRight => (moved.x - start.x, (moved.y + moved.height) - (start.y + start.height)),
            _ => ((moved.x + moved.width) - (start.x + start.width), moved.y - start.y),
        };
        assert!(anchor_x.abs() < 1e-9 && anchor_y.abs() < 1e-9, "{name}: the opposite corner stays");
    }
    let smallest = crop_drag(start, setup(size, shape), CropHandle::BottomRight, -9.0, -9.0);
    let largest = crop_initial(setup(size, shape), None);
    assert!((smallest.width - largest.width / 3.0).abs() < 1e-9, "the smallest frame is a third of the largest");
}

#[test]
fn inv_crop_07_the_finish_refuses_a_rectangle_that_is_not_a_number_too_large_too_small_or_out_of_proportion() {
    let size = photo(2000, 3000);
    let good = crop_initial(setup(size, chat()), None);
    assert_eq!(crop_finish(good, setup(size, chat())), Some(good));
    let shifted = crop_finish(rect(good.x + 0.9, good.y, good.width, good.height), setup(size, chat())).unwrap();
    near(shifted, rect(1.0 - good.width, good.y, good.width, good.height), "a rectangle past the edge is moved in");
    for (name, bad, aspect) in [
        ("not a number", rect(f64::NAN, 0.0, 0.5, 0.5), None),
        ("infinite", rect(0.0, 0.0, f64::INFINITY, 0.5), None),
        ("empty", rect(0.2, 0.2, 0.0, 0.5), None),
        ("larger than the photo", rect(0.0, 0.0, 1.2, 1.0), None),
        ("below the smallest side", rect(0.0, 0.0, 0.01, 0.01), None),
        ("out of proportion", rect(0.0, 0.0, good.width * 1.05, good.height), chat()),
        ("a third too small with proportions", rect(0.0, 0.0, good.width / 3.5, good.height / 3.5), chat()),
    ] {
        assert_eq!(crop_finish(bad, setup(size, aspect)), None, "{name}");
    }
    assert_eq!(crop_finish(good, setup(photo(0, 10), chat())), None);
    let rounded = rect(good.x, good.y, good.width - 0.4 / 2000.0, good.height);
    assert!(crop_finish(rounded, setup(size, chat())).is_some(), "less than a pixel of drift is inside the tolerance");
}

#[test]
fn inv_crop_08_a_finished_rectangle_is_a_plan_that_cuts_the_proportions_and_every_pixel_axis_is_not_empty() {
    for (name, size, aspect, output) in [("chat", photo(4032, 3024), chat(), (1080, 1920)), ("square", photo(3024, 4032), square(), (1080, 1080)), ("free", photo(1000, 700), None, (500, 350))] {
        let mut frame = crop_initial(setup(size, aspect), None);
        frame = crop_zoom(frame, setup(size, aspect), 1.7, 0.4, 0.6);
        frame = crop_pan(frame, 0.013, -0.021);
        let finished = crop_finish(frame, setup(size, aspect)).unwrap_or_else(|| panic!("{name}: {frame:?}"));
        let edit = PhotoEdit { max_size: None, enlarge: false, crop: Some(finished), output_size: Some(photo(output.0, output.1)), mirror: false, rotation_degrees: 0 };
        let plan = photo_plan(size.width, size.height, 1, edit).unwrap_or_else(|| panic!("{name}"));
        assert!(plan.crop.width > 0 && plan.crop.height > 0, "{name}");
        if let Some(shape) = aspect {
            let error = (i64::from(plan.crop.width) * i64::from(shape.height) - i64::from(plan.crop.height) * i64::from(shape.width)).abs();
            assert!(error <= i64::from(shape.width + shape.height), "{name}: {plan:?}");
        }
    }
}

#[test]
fn inv_crop_09_a_quarter_turn_turns_the_rectangle_with_the_photo_and_a_fixed_frame_is_fitted_again() {
    let size = photo(2000, 1000);
    let start = rect(0.1, 0.2, 0.3, 0.4);
    near(crop_turn(start, setup(size, None), true), rect(1.0 - 0.2 - 0.4, 0.1, 0.4, 0.3), "clockwise");
    near(crop_turn(start, setup(size, None), false), rect(0.2, 1.0 - 0.1 - 0.3, 0.4, 0.3), "counter clockwise");
    let there = crop_turn(start, setup(size, None), true);
    near(crop_turn(there, setup(photo(1000, 2000), None), false), start, "back again");
    let turned = photo(1000, 2000);
    let fixed = crop_turn(crop_initial(setup(size, chat()), None), setup(size, chat()), true);
    assert!(inside(fixed));
    assert!((pixels_wide_per_high(fixed, turned) - 1080.0 / 1920.0).abs() < 1e-9);
    let free = crop_turn(rect(f64::NAN, 0.0, 1.0, 1.0), setup(size, None), true);
    near(free, rect(0.0, 0.0, 1.0, 1.0), "a broken rectangle starts again");
}

#[test]
fn inv_crop_10_the_view_maps_gestures_to_fractions_one_way_for_both_platforms() {
    let size = photo(2000, 3000);
    let first = crop_initial(setup(size, chat()), None);
    let layout = crop_layout(first, setup(size, chat()), view()).unwrap();
    assert!((layout.frame.width / layout.frame.height - 1080.0 / 1920.0).abs() < 1e-9);
    assert!((layout.frame.x * 2.0 + layout.frame.width - 400.0).abs() < 1e-9 && (layout.frame.y * 2.0 + layout.frame.height - 800.0).abs() < 1e-9, "the frame is in the middle");
    assert!((layout.photo.x + first.x * layout.photo.width - layout.frame.x).abs() < 1e-9, "the frame shows the rectangle of the photo");
    let dragged = crop_pan_view(first, setup(size, chat()), view(), -layout.photo.width * 0.1, 0.0);
    assert!((dragged.x - (first.x + 0.1)).abs() < 1e-9 || dragged.x + dragged.width >= 1.0 - 1e-9, "the photo follows the finger, the frame goes the other way");
    let free_first = rect(0.2, 0.2, 0.5, 0.5);
    let free_layout = crop_layout(free_first, setup(size, None), view()).unwrap();
    assert!((free_layout.photo.width - 400.0).abs() < 1e-9, "the photo fits the view");
    let free_moved = crop_pan_view(free_first, setup(size, None), view(), 40.0, 0.0);
    assert!((free_moved.x - (0.2 + 0.1)).abs() < 1e-9, "the free frame follows the finger");
    let zoomed = crop_zoom_view(first, setup(size, chat()), view(), 2.0, layout.frame.x + layout.frame.width / 2.0, layout.frame.y + layout.frame.height / 2.0);
    assert!((zoomed.width - first.width / 2.0).abs() < 1e-9);
    let handled = crop_drag_view(free_first, setup(size, None), view(), CropHandle::Right, 40.0, 0.0);
    assert!((handled.width - 0.6).abs() < 1e-9);
    for (name, x, y, expected) in [
        ("corner", 100.0, 100.0, Some(CropHandle::TopLeft)),
        ("far corner", 300.0, 200.0, Some(CropHandle::BottomRight)),
        ("left side", 100.0, 150.0, Some(CropHandle::Left)),
        ("top side", 200.0, 100.0, Some(CropHandle::Top)),
        ("inside", 200.0, 150.0, None),
        ("outside", 10.0, 10.0, None),
    ] {
        let frame = CropBox { x: 100.0, y: 100.0, width: 200.0, height: 100.0 };
        assert_eq!(crop_handle_at(frame, x, y, 12.0), expected, "{name}");
    }
    assert_eq!(crop_layout(first, setup(size, chat()), CropView { width: 10.0, height: 10.0, inset: 8.0 }), None);
}

#[test]
fn inv_crop_16_a_finished_gesture_is_a_pinch_when_it_scaled_a_pan_when_it_only_moved_and_the_change_flag_never_clears() {
    for (name, panned, zoomed, was, kind, now) in [
        ("none", false, false, false, None, false),
        ("pan", true, false, false, Some(CropGesture::Pan), true),
        ("pinch", false, true, false, Some(CropGesture::Pinch), true),
        ("pan and pinch", true, true, false, Some(CropGesture::Pinch), true),
        ("none after a change", false, false, true, None, true),
    ] {
        assert_eq!(crop_gesture_end(panned, zoomed, was), CropGestureEnd { kind, was_position_changed: now }, "{name}");
    }
}

#[test]
fn inv_crop_17_the_telemetry_is_the_scale_and_the_move_of_the_frame_against_the_first_frame_in_view_points() {
    let size = photo(2000, 3000);
    let fixed = setup(size, chat());
    let first = crop_initial(fixed, None);
    let layout = crop_layout(first, fixed, view()).unwrap();
    let (middle_x, middle_y) = (layout.frame.x + layout.frame.width / 2.0, layout.frame.y + layout.frame.height / 2.0);
    let panned = crop_pan_view(first, fixed, view(), 30.0, 0.0);
    let pinched = crop_zoom_view(first, fixed, view(), 2.0, middle_x, middle_y);
    let both = crop_zoom_view(panned, fixed, view(), 2.0, middle_x, middle_y);
    let free = setup(size, None);
    let free_first = rect(0.2, 0.2, 0.5, 0.5);
    let free_panned = crop_pan_view(free_first, free, view(), 40.0, 0.0);
    for (name, setup, baseline, now, was, expected) in [
        ("no gesture", fixed, first, first, false, (1.0, 0, 0, false)),
        ("pan", fixed, first, panned, true, (1.0, 30, 0, true)),
        ("pinch", fixed, first, pinched, true, (2.0, 0, 0, true)),
        ("pan and pinch", fixed, first, both, true, (2.0, 60, 0, true)),
        ("moved back keeps the flag", fixed, first, first, true, (1.0, 0, 0, true)),
        ("free frame follows the finger", free, free_first, free_panned, true, (1.0, 40, 0, true)),
    ] {
        let telemetry = crop_telemetry(baseline, now, setup, view(), was);
        assert_eq!((telemetry.final_scale, telemetry.final_translate_x, telemetry.final_translate_y, telemetry.was_position_changed), expected, "{name}");
    }
    let unplaced = crop_telemetry(first, pinched, fixed, CropView { width: 0.0, height: 0.0, inset: 0.0 }, true);
    assert_eq!((unplaced.final_scale, unplaced.final_translate_x, unplaced.final_translate_y), (2.0, 0, 0), "a view that cannot be laid out gives no move");
}

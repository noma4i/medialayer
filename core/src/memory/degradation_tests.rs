use super::*;
use crate::pool::Pressure;

const CEILING: u64 = 1000;

fn step_at(used: u64) -> Step {
    Degradation::default().for_pressure(used, CEILING, None)
}

#[test]
fn load_maps_to_the_ladder() {
    assert_eq!(step_at(0), Step::Normal);
    assert_eq!(step_at(699), Step::Normal);
    assert_eq!(step_at(700), Step::LowerQuality);
    assert_eq!(step_at(800), Step::ShorterBuffer);
    assert_eq!(step_at(880), Step::FewerSlots);
    assert_eq!(step_at(940), Step::PlayingSlotOnly);
    assert_eq!(step_at(990), Step::PlaceholdersOnly);
    assert_eq!(step_at(1000), Step::PlaceholdersOnly);
}

#[test]
fn a_zero_ceiling_is_placeholders_only() {
    assert_eq!(Degradation::default().for_pressure(0, 0, None), Step::PlaceholdersOnly);
}

#[test]
fn the_step_never_decreases_as_load_grows() {
    let mut previous = Step::Normal;
    for used in 0..=CEILING {
        let step = step_at(used);
        assert!(step >= previous, "step fell at {used}");
        previous = step;
    }
}

#[test]
fn the_step_never_decreases_as_system_pressure_grows() {
    for used in (0..=CEILING).step_by(10) {
        let none = Degradation::default().for_pressure(used, CEILING, None);
        let moderate = Degradation::default().for_pressure(used, CEILING, Some(Pressure::Moderate));
        let critical = Degradation::default().for_pressure(used, CEILING, Some(Pressure::Critical));
        assert!(none <= moderate && moderate <= critical, "at {used}");
    }
}

#[test]
fn system_pressure_shifts_the_load_step_up_and_removal_lifts_back() {
    let mut degradation = Degradation::default();
    assert_eq!(degradation.for_pressure(0, CEILING, None), Step::Normal);
    assert_eq!(degradation.for_pressure(0, CEILING, Some(Pressure::Moderate)), Step::LowerQuality);
    assert_eq!(degradation.for_pressure(0, CEILING, Some(Pressure::Critical)), Step::ShorterBuffer);
    assert_eq!(degradation.for_pressure(0, CEILING, None), Step::Normal);
    assert_eq!(Degradation::default().for_pressure(700, CEILING, Some(Pressure::Moderate)), Step::ShorterBuffer);
    assert_eq!(Degradation::default().for_pressure(995, CEILING, Some(Pressure::Critical)), Step::PlaceholdersOnly);
}

#[test]
fn hysteresis_holds_a_step_inside_the_margin() {
    let mut degradation = Degradation::default();
    assert_eq!(degradation.for_pressure(720, CEILING, None), Step::LowerQuality);
    assert_eq!(degradation.for_pressure(680, CEILING, None), Step::LowerQuality);
    assert_eq!(degradation.for_pressure(651, CEILING, None), Step::LowerQuality);
    assert_eq!(degradation.for_pressure(649, CEILING, None), Step::Normal);
}

#[test]
fn recovery_walks_down_to_the_highest_step_the_load_still_justifies() {
    let mut degradation = Degradation::default();
    assert_eq!(degradation.for_pressure(995, CEILING, None), Step::PlaceholdersOnly);
    assert_eq!(degradation.for_pressure(820, CEILING, None), Step::ShorterBuffer);
    assert_eq!(degradation.for_pressure(0, CEILING, None), Step::Normal);
}

#[test]
fn the_margin_is_tunable() {
    let tuning = DegradationTuning { margin_percent: 0, ..DegradationTuning::default() };
    let mut degradation = Degradation::new(tuning);
    assert_eq!(degradation.for_pressure(720, CEILING, None), Step::LowerQuality);
    assert_eq!(degradation.for_pressure(699, CEILING, None), Step::Normal);
}

#[test]
fn plans_shrink_monotonically_down_the_ladder() {
    let steps = [Step::Normal, Step::LowerQuality, Step::ShorterBuffer, Step::FewerSlots, Step::PlayingSlotOnly, Step::PlaceholdersOnly];
    for pair in steps.windows(2) {
        let (before, after) = (StepPlan::for_step(pair[0]), StepPlan::for_step(pair[1]));
        assert!(after.buffer_seconds(4) <= before.buffer_seconds(4));
        assert!(after.slot_limit(6) <= before.slot_limit(6));
        assert!(after.reach_limit(2) <= before.reach_limit(2));
        assert!(after.admission_percent <= before.admission_percent);
        assert!(after.bitrate_cap_kbps.unwrap_or(u32::MAX) <= before.bitrate_cap_kbps.unwrap_or(u32::MAX));
    }
}

#[test]
fn plan_values_per_step() {
    let normal = StepPlan::for_step(Step::Normal);
    assert_eq!((normal.bitrate_cap_kbps, normal.buffer_seconds(4), normal.slot_limit(6), normal.reach_limit(2)), (None, 4, 6, 2));
    let fewer = StepPlan::for_step(Step::FewerSlots);
    assert_eq!((fewer.slot_limit(6), fewer.reach_limit(2)), (3, 1));
    assert_eq!(StepPlan::for_step(Step::FewerSlots).slot_limit(2), 2);
    let playing = StepPlan::for_step(Step::PlayingSlotOnly);
    assert_eq!((playing.slot_limit(6), playing.reach_limit(2)), (1, 0));
    assert_eq!(playing.buffer_seconds(1), 1);
    let none = StepPlan::for_step(Step::PlaceholdersOnly);
    assert_eq!((none.slot_limit(6), none.reach_limit(2), none.buffer_seconds(4)), (0, 0, 0));
}

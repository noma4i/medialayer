use super::*;

const RATE: u32 = 44_100;
const AAC: u32 = 1024;
const FRAME_US: i64 = 23_220;

fn config() -> CoverageConfig {
    CoverageConfig { audio_required: true, audio_track: true, sample_rate: RATE, frame_samples: AAC }
}

fn packet(start_us: i64, end_us: i64) -> AudioInterval {
    AudioInterval { start_us, end_us, samples: ((end_us - start_us) * RATE as i64 / 1_000_000) as u64 }
}

/// Back to back AAC frames from `start_us`, as many whole ones as fit before `end_us`.
fn frames(start_us: i64, end_us: i64) -> Vec<AudioInterval> {
    let mut out = Vec::new();
    let mut at = start_us;
    let mut index = 0i64;
    loop {
        let from = start_us + index * 1024 * 1_000_000 / RATE as i64;
        let to = start_us + (index + 1) * 1024 * 1_000_000 / RATE as i64;
        if to > end_us {
            break;
        }
        out.push(AudioInterval { start_us: from, end_us: to, samples: 1024 });
        at = to;
        index += 1;
    }
    let _ = at;
    out
}

fn video(end_us: i64) -> Option<VideoBounds> {
    Some(VideoBounds { start_us: 0, end_us })
}

fn read(config: CoverageConfig, intervals: &[AudioInterval], video: Option<VideoBounds>) -> AudioCoverage {
    let scan = intervals.iter().fold(coverage_begin(config), |scan, interval| coverage_push(scan, *interval));
    coverage_finish(scan, video)
}

#[test]
fn inv_coverage_01_real_clips_give_the_verdict_of_the_read_pcm() {
    let mut hole = frames(0, 800_000);
    let hole_at = hole.last().unwrap().end_us;
    hole.extend(frames(hole_at + 400_000, 2_000_000));
    let one_frame = {
        let mut all = frames(0, 2_000_000);
        all.remove(20);
        all
    };
    let off = CoverageConfig { audio_required: false, audio_track: false, ..config() };
    let no_track = CoverageConfig { audio_track: false, ..config() };
    let zero_packet = [AudioInterval { start_us: 0, end_us: 0, samples: 0 }];
    let table: Vec<(&str, CoverageConfig, Vec<AudioInterval>, Option<VideoBounds>, CoverageVerdict)> = vec![
        ("whole audio", config(), frames(0, 2_000_000), video(2_000_000), CoverageVerdict::Covered),
        ("AAC trimmed, ends short of the video by under one frame", config(), frames(10_000, 1_990_000), video(2_000_000), CoverageVerdict::Covered),
        ("audio only", config(), frames(0, 2_000_000), None, CoverageVerdict::Covered),
        ("audio off, no track", off, Vec::new(), video(2_000_000), CoverageVerdict::Skipped),
        ("video only while sound is asked", no_track, Vec::new(), video(2_000_000), CoverageVerdict::Missing),
        ("nothing read", config(), Vec::new(), video(2_000_000), CoverageVerdict::Empty),
        ("a packet of zero length and no sample", config(), zero_packet.to_vec(), video(2_000_000), CoverageVerdict::Empty),
        ("a hole in the middle", config(), hole, video(2_000_000), CoverageVerdict::MidGap),
        ("one lost AAC frame in the middle", config(), one_frame, video(2_000_000), CoverageVerdict::MidGap),
        ("sound dies half way", config(), frames(0, 1_000_000), video(2_000_000), CoverageVerdict::TailCut),
        ("sound starts late", config(), frames(600_000, 2_000_000), video(2_000_000), CoverageVerdict::HeadGap),
        ("empty video range", config(), frames(0, 2_000_000), video(0), CoverageVerdict::Broken),
    ];
    for (name, config, intervals, bounds, want) in table {
        assert_eq!(read(config, &intervals, bounds).verdict, want, "{name}");
    }
}

#[test]
fn inv_coverage_02_untrusted_intervals_are_broken_and_give_a_refusal() {
    let good = frames(0, 2_000_000);
    let mut repeat = good.clone();
    repeat.insert(6, good[5]);
    let mut overlap = good.clone();
    overlap[8].start_us -= 5_000;
    let reversed = vec![good[1], good[0]];
    let mut buffer = good.clone();
    buffer[4] = AudioInterval { samples: 1, ..buffer[4] };
    let mut silent = good.clone();
    silent[4] = AudioInterval { samples: 0, ..silent[4] };
    let backwards = vec![AudioInterval { start_us: 500_000, end_us: 400_000, samples: 4410 }];
    let table: Vec<(&str, CoverageConfig, Vec<AudioInterval>)> = vec![
        ("a repeated interval", config(), repeat),
        ("an overlap", config(), overlap),
        ("the first two packets in reverse order", config(), reversed),
        ("a 23 ms buffer with one sample", config(), buffer),
        ("positive length and no sample", config(), silent),
        ("ends before it starts", config(), backwards),
        ("zero rate", CoverageConfig { sample_rate: 0, ..config() }, good.clone()),
        ("zero frame", CoverageConfig { frame_samples: 0, ..config() }, good),
    ];
    for (name, config, intervals) in table {
        let got = read(config, &intervals, video(2_000_000));
        assert_eq!((got.verdict, audio_coverage_refusal(got.verdict)), (CoverageVerdict::Broken, Some(CameraRefusal::MicrophoneSilent)), "{name}");
    }
    let huge = AudioInterval { start_us: 0, end_us: 1_000_000, samples: u64::MAX };
    let overflow = coverage_push(coverage_push(coverage_begin(CoverageConfig { sample_rate: 1_000_000, ..config() }), AudioInterval { samples: 1_000_000, ..huge }), AudioInterval { start_us: 1_000_000, end_us: 2_000_000, samples: 1_000_000 });
    assert_eq!(coverage_finish(overflow, None).verdict, CoverageVerdict::Covered);
    let wrapped = coverage_push(CoverageScan { samples: u64::MAX, packets: 1, reached_us: 1_000_000, first_us: Some(0), ..coverage_begin(CoverageConfig { sample_rate: 1_000_000, ..config() }) }, AudioInterval { start_us: 1_000_000, end_us: 2_000_000, samples: 1_000_000 });
    assert_eq!(coverage_finish(wrapped, None).verdict, CoverageVerdict::Broken);
}

#[test]
fn inv_coverage_03_the_result_names_the_first_defect_and_the_real_counts() {
    let mut lost = frames(0, 1_000_000);
    let hole_at = lost.last().unwrap().end_us;
    lost.extend(frames(hole_at + 2 * FRAME_US, 2_000_000));
    let got = read(config(), &lost, video(2_000_000));
    assert_eq!((got.verdict, got.gap_at_us, got.gap_us, got.samples, got.first_us), (CoverageVerdict::MidGap, Some(hole_at), Some(2 * FRAME_US), lost.len() as u64 * 1024, Some(0)));
    let tail = read(config(), &frames(0, 1_000_000), video(2_000_000));
    assert_eq!((tail.verdict, tail.gap_at_us, tail.gap_us), (CoverageVerdict::TailCut, tail.last_us, Some(2_000_000 - tail.last_us.unwrap())));
    let head = read(config(), &frames(600_000, 2_000_000), video(2_000_000));
    assert_eq!((head.verdict, head.gap_at_us, head.gap_us, head.first_us), (CoverageVerdict::HeadGap, Some(0), Some(600_000), Some(600_000)));
    let mut both = lost.clone();
    both.push(AudioInterval { start_us: 0, end_us: 10_000, samples: 441 });
    let first = read(config(), &both, video(2_000_000));
    assert_eq!((first.verdict, first.gap_at_us), (CoverageVerdict::MidGap, Some(hole_at)), "the first defect stays");
    // plain PCM: the step is one sample, ten samples lost in the middle are a gap, one sample of rounding is not.
    let pcm = CoverageConfig { frame_samples: 1, ..config() };
    let split = |gap: i64| [packet(0, 500_000), packet(500_000 + gap, 1_000_000)];
    assert_eq!((read(pcm, &split(0), None).verdict, read(pcm, &split(1), None).verdict, read(pcm, &split(1_000), None).verdict), (CoverageVerdict::Covered, CoverageVerdict::Covered, CoverageVerdict::MidGap));
}

#[test]
fn inv_coverage_04_a_scan_changed_from_outside_or_run_over_is_broken_and_never_panics() {
    let ready = coverage_push(coverage_begin(config()), frames(0, 100_000)[0]);
    let table: Vec<(&str, CoverageScan)> = vec![
        ("rate zero", CoverageScan { config: CoverageConfig { sample_rate: 0, ..config() }, ..coverage_begin(config()) }),
        ("frame zero", CoverageScan { config: CoverageConfig { frame_samples: 0, ..config() }, ..coverage_begin(config()) }),
        ("packets with no first", CoverageScan { first_us: None, ..ready }),
        ("packets with no samples", CoverageScan { samples: 0, ..ready }),
        ("first after reached", CoverageScan { first_us: Some(ready.reached_us + 1), ..ready }),
        ("samples with no packets", CoverageScan { packets: 0, ..ready }),
        ("packet count at the end", CoverageScan { packets: u64::MAX, ..ready }),
        ("sample count at the end", CoverageScan { samples: u64::MAX, ..ready }),
    ];
    for (name, scan) in table {
        let next = coverage_push(scan, AudioInterval { start_us: ready.reached_us, end_us: ready.reached_us + 23_220, samples: 1024 });
        assert_eq!((next.defect, coverage_finish(next, None).verdict), (Some(CoverageVerdict::Broken), CoverageVerdict::Broken), "{name}");
        if name.starts_with("rate") || name.starts_with("frame") || name.contains("first") || name.contains("no ") {
            assert_eq!(coverage_finish(scan, None).verdict, CoverageVerdict::Broken, "{name} at finish");
        }
    }
}

#[test]
fn inv_coverage_05_only_a_lost_sound_gives_a_refusal() {
    for (v, want) in [
        (CoverageVerdict::Skipped, None),
        (CoverageVerdict::Covered, None),
        (CoverageVerdict::Missing, Some(CameraRefusal::MicrophoneSilent)),
        (CoverageVerdict::Empty, Some(CameraRefusal::MicrophoneSilent)),
        (CoverageVerdict::Broken, Some(CameraRefusal::MicrophoneSilent)),
        (CoverageVerdict::HeadGap, Some(CameraRefusal::MicrophoneSilent)),
        (CoverageVerdict::MidGap, Some(CameraRefusal::MicrophoneSilent)),
        (CoverageVerdict::TailCut, Some(CameraRefusal::MicrophoneSilent)),
    ] {
        assert_eq!(audio_coverage_refusal(v), want, "{v:?}");
    }
}

/// AAC frames of `rate` whose boundaries are rounded to a whole microsecond the way a container reports them, `lost` samples missing after packet `at`.
fn rounded_frames(rate: u32, count: usize, jitter: &mut dyn FnMut() -> i64, hole: Option<(usize, i64)>) -> Vec<AudioInterval> {
    let at_us = |sample: i64| (sample * 1_000_000 + rate as i64 / 2) / rate as i64;
    let mut sample = 0i64;
    let mut out = Vec::new();
    for index in 0..count {
        if let Some((at, lost)) = hole {
            if index == at { sample += lost; }
        }
        let start = at_us(sample) + jitter();
        let end = at_us(sample + 1024) + jitter();
        out.push(AudioInterval { start_us: start, end_us: end, samples: 1024 });
        sample += 1024;
    }
    out
}

#[test]
fn inv_coverage_06_a_joint_is_judged_in_samples_of_the_track_rate() {
    for rate in [44_100u32, 48_000] {
        let config = CoverageConfig { sample_rate: rate, ..config() };
        let sample_us = 1_000_000 / rate as i64;
        let at = |us: i64| -> Vec<AudioInterval> {
            let first = AudioInterval { start_us: 0, end_us: 1_000_000, samples: rate as u64 };
            vec![first, AudioInterval { start_us: 1_000_000 + us, end_us: 2_000_000 + us, samples: rate as u64 }]
        };
        let one_sample = (1_000_000 + rate as i64 / 2) / rate as i64 + 1;
        assert_eq!(read(config, &at(0), None).verdict, CoverageVerdict::Covered, "{rate}: a joint");
        assert_eq!(read(config, &at(one_sample), None).verdict, CoverageVerdict::Covered, "{rate}: one sample and a microsecond of rounding");
        assert_eq!(read(config, &at(one_sample + sample_us), None).verdict, CoverageVerdict::MidGap, "{rate}: two samples are a hole");
        assert_eq!(read(config, &at(-one_sample), None).verdict, CoverageVerdict::Covered, "{rate}: one sample of overlap");
        assert_eq!(read(config, &at(-(one_sample + sample_us)), None).verdict, CoverageVerdict::Broken, "{rate}: two samples of overlap");
    }
    // The 24 microsecond joint a real clip showed on the emulator at 44100 Hz.
    let real = [AudioInterval { start_us: 0, end_us: 513_309, samples: 22_637 }, AudioInterval { start_us: 513_333, end_us: 534_100, samples: 916 }];
    assert_eq!(read(config(), &real, None).verdict, CoverageVerdict::Covered);
}

#[test]
fn inv_coverage_07_random_clips_with_rounded_boundaries_are_covered_and_a_hole_of_two_samples_is_not() {
    let name = "inv_coverage_07_random_clips_with_rounded_boundaries_are_covered_and_a_hole_of_two_samples_is_not";
    let mut dice = crate::seeded::Dice::new(name);
    for round in 0..2_000 {
        let rate = [8_000u32, 16_000, 22_050, 44_100, 48_000][dice.index(5)];
        let config = CoverageConfig { sample_rate: rate, ..config() };
        let count = dice.between(3, 40) as usize;
        let rounding = |dice: &mut crate::seeded::Dice| dice.between(0, 2) as i64 - 1;
        let whole = {
            let mut jitter = || rounding(&mut dice);
            rounded_frames(rate, count, &mut jitter, None)
        };
        assert_eq!(read(config, &whole, None).verdict, CoverageVerdict::Covered, "round {rate} Hz, {count} packets: {whole:?}\nreproduce: ML_SEED={}", crate::seeded::seed());
        let at = dice.between(1, count as u64 - 1) as usize;
        let one = {
            let mut jitter = || rounding(&mut dice);
            rounded_frames(rate, count, &mut jitter, Some((at, 1)))
        };
        assert_eq!(read(config, &one, None).verdict, CoverageVerdict::Covered, "round {round}: {rate} Hz, one sample lost after packet {at}: {one:?}\nreproduce: ML_SEED={}", crate::seeded::seed());
        let lost = dice.between(2, 40) as i64;
        let holed = {
            let mut jitter = || rounding(&mut dice);
            rounded_frames(rate, count, &mut jitter, Some((at, lost)))
        };
        assert_eq!(read(config, &holed, None).verdict, CoverageVerdict::MidGap, "round {round}: {rate} Hz, {lost} samples lost after packet {at}: {holed:?}\nreproduce: ML_SEED={}", crate::seeded::seed());
    }
}

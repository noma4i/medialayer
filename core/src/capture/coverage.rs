use super::refusal::CameraRefusal;

const MICROS: i128 = 1_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct AudioInterval {
    pub start_us: i64,
    pub end_us: i64,
    pub samples: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct VideoBounds {
    pub start_us: i64,
    pub end_us: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct CoverageConfig {
    pub audio_required: bool,
    pub audio_track: bool,
    pub sample_rate: u32,
    pub frame_samples: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum CoverageVerdict {
    Skipped,
    Covered,
    Missing,
    Empty,
    Broken,
    HeadGap,
    MidGap,
    TailCut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct CoverageScan {
    pub config: CoverageConfig,
    pub first_us: Option<i64>,
    pub reached_us: i64,
    pub samples: u64,
    pub packets: u64,
    pub defect: Option<CoverageVerdict>,
    pub gap_at_us: Option<i64>,
    pub gap_us: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct AudioCoverage {
    pub verdict: CoverageVerdict,
    pub samples: u64,
    pub first_us: Option<i64>,
    pub last_us: Option<i64>,
    pub gap_at_us: Option<i64>,
    pub gap_us: Option<i64>,
}

fn sound(scan: &CoverageScan) -> bool {
    let config = scan.config;
    let counted = if scan.packets == 0 { scan.first_us.is_none() && scan.samples == 0 && scan.reached_us == 0 } else { scan.samples > 0 && scan.first_us.is_some_and(|first| first <= scan.reached_us) };
    config.sample_rate > 0 && config.frame_samples > 0 && counted
}

/// The samples a span of microseconds holds, rounded to the nearest: a boundary rounded to a microsecond is off by less than half a sample at any audio rate.
fn samples_in(us: i128, rate: u32) -> i128 {
    (us * rate as i128 + MICROS / 2).div_euclid(MICROS)
}

fn ceil_us(samples: i128, rate: u32) -> i128 {
    (samples * MICROS + rate as i128 - 1) / rate as i128
}

#[uniffi::export]
pub fn coverage_begin(config: CoverageConfig) -> CoverageScan {
    let defect = if !config.audio_required {
        Some(CoverageVerdict::Skipped)
    } else if !config.audio_track {
        Some(CoverageVerdict::Missing)
    } else if config.sample_rate == 0 || config.frame_samples == 0 {
        Some(CoverageVerdict::Broken)
    } else {
        None
    };
    CoverageScan { config, first_us: None, reached_us: 0, samples: 0, packets: 0, defect, gap_at_us: None, gap_us: None }
}

/// One rule for the audio of a recorded clip on both platforms: inside the stream one PCM sample of rounding is allowed, counted in samples of the track rate (a longer hole is `MidGap`, an overlap, a repeat or a count that does not fit its length is `Broken`), the ends may differ from the video by one native codec frame. The first defect stays, a scan changed from outside is `Broken`.
#[uniffi::export]
pub fn coverage_push(scan: CoverageScan, interval: AudioInterval) -> CoverageScan {
    let mut next = scan;
    if scan.defect.is_some_and(|defect| defect != CoverageVerdict::MidGap) {
        return next;
    }
    if !sound(&scan) {
        next.defect = scan.defect.or(Some(CoverageVerdict::Broken));
        return next;
    }
    let sample_us = ceil_us(1, scan.config.sample_rate);
    let length = interval.end_us as i128 - interval.start_us as i128;
    if length == 0 && interval.samples == 0 {
        return next;
    }
    let heard = interval.samples as i128 * MICROS / scan.config.sample_rate as i128;
    let step = samples_in(interval.start_us as i128 - scan.reached_us as i128, scan.config.sample_rate);
    let total = scan.samples.checked_add(interval.samples).zip(scan.packets.checked_add(1));
    let broken = length <= 0 || interval.samples == 0 || (length - heard).abs() > sample_us || (scan.packets > 0 && step < -1) || total.is_none();

    if broken {
        next.defect = scan.defect.or(Some(CoverageVerdict::Broken));
        return next;
    }
    if scan.packets > 0 && step > 1 && next.defect.is_none() {
        next.defect = Some(CoverageVerdict::MidGap);
        next.gap_at_us = Some(scan.reached_us);
        next.gap_us = Some(interval.start_us.saturating_sub(scan.reached_us));
    }
    next.first_us = scan.first_us.or(Some(interval.start_us));
    next.reached_us = interval.end_us;
    if let Some((samples, packets)) = total {
        next.samples = samples;
        next.packets = packets;
    }
    next
}

#[uniffi::export]
pub fn coverage_finish(scan: CoverageScan, video: Option<VideoBounds>) -> AudioCoverage {
    let mut gap = (scan.gap_at_us, scan.gap_us);
    let verdict = match scan.defect {
        Some(CoverageVerdict::MidGap) if !sound(&scan) => CoverageVerdict::Broken,
        Some(defect) => defect,
        None if !sound(&scan) => CoverageVerdict::Broken,
        None if scan.packets == 0 => CoverageVerdict::Empty,
        None => match video {
            Some(video) if video.end_us <= video.start_us => CoverageVerdict::Broken,
            Some(video) => {
                let frame_us = ceil_us(scan.config.frame_samples as i128, scan.config.sample_rate);
                let head = scan.first_us.unwrap_or(0) as i128 - video.start_us as i128;
                let tail = video.end_us as i128 - scan.reached_us as i128;
                if head > frame_us {
                    gap = (Some(video.start_us), i64::try_from(head).ok());
                    CoverageVerdict::HeadGap
                } else if tail > frame_us {
                    gap = (Some(scan.reached_us), i64::try_from(tail).ok());
                    CoverageVerdict::TailCut
                } else {
                    CoverageVerdict::Covered
                }
            }
            None => CoverageVerdict::Covered,
        },
    };
    AudioCoverage { verdict, samples: scan.samples, first_us: scan.first_us, last_us: (scan.packets > 0).then_some(scan.reached_us), gap_at_us: gap.0, gap_us: gap.1 }
}

#[uniffi::export]
pub fn audio_coverage_refusal(verdict: CoverageVerdict) -> Option<CameraRefusal> {
    match verdict {
        CoverageVerdict::Skipped | CoverageVerdict::Covered => None,
        _ => Some(CameraRefusal::MicrophoneSilent),
    }
}

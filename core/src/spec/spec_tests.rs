use super::*;
use crate::limits::{EvictionStrategy, Limits, LimitsError};

fn sample() -> MediaSpec {
    let mut video = VideoSpec::new("moment_video");
    video.cover_field = Some("coverUrl".to_string());
    video.prefetch = PrefetchSpec { from: 1, to: 4, depth: 3 };
    video.pool = PoolSpec { slots: 4, reach: 2, hold_off: 5, trim_delay: 60 };
    video.evict = Some(Evict::Lfu);
    MediaSpec {
        cache: CacheSpec {
            budget: 2 * 1024 * 1024 * 1024,
            shrink_to: 80,
            evict: Evict::ExpireAfter { seconds: 604800 },
            namespace_headers: vec!["Authorization".to_string()],
            ..CacheSpec::default()
        },
        videos: vec![video],
        images: vec![ImageSpec {
            name: "avatar".to_string(),
            blurhash_field: Some("blurHash".to_string()),
            size: Some((288, 288)),
            prefetch_window: Some((1, 3)),
            evict: None,
        }],
        audios: vec![AudioSpec { name: "voice_note".to_string(), evict: Some(Evict::Fifo) }],
    }
}

#[test]
fn json_is_stable_and_round_trips() {
    let spec = sample();
    let json = spec.to_json();
    assert_eq!(MediaSpec::from_json(&json).unwrap(), spec);
    assert_eq!(MediaSpec::from_json(&json).unwrap().to_json(), json);
    assert!(json.starts_with(r#"{"cache":{"budget":2147483648,"min":67108864,"shrink_to":80,"evict":{"expire_after":{"seconds":604800}}"#));
}

#[test]
fn json_with_an_unknown_option_is_rejected() {
    for text in [r#"{"cache":{"bogus":1}}"#, r#"{"videos":[{"name":"a","bogus":1}]}"#, r#"{"images":[{"name":"a","prefetch_window":[1,2,3]}]}"#] {
        assert!(MediaSpec::from_json(text).is_err(), "{text}");
    }
}

#[test]
fn limits_take_cache_values_and_the_first_video_and_default_to_core_limits() {
    assert_eq!(MediaSpec::default().limits(), Limits::default());
    let limits = sample().limits();
    assert_eq!(limits.budget, 2 * 1024 * 1024 * 1024);
    assert_eq!(limits.shrink_to, 0.8);
    assert_eq!(limits.strategy, EvictionStrategy::ExpireAfter(604800));
    assert_eq!((limits.slots, limits.prefetch_from, limits.prefetch_to, limits.depth_seconds), (4, 1, 4, 3));
    assert_eq!(limits.timeout_seconds, 30);
}

#[test]
fn validate_rejects_bad_specs() {
    let cases: Vec<(&str, Box<dyn Fn(&mut MediaSpec)>, SpecError)> = vec![
        (
            "duplicate across kinds",
            Box::new(|spec| spec.audios.push(AudioSpec { name: "moment_video".to_string(), evict: None })),
            SpecError::DuplicateName("moment_video".to_string()),
        ),
        (
            "budget below min",
            Box::new(|spec| spec.cache.budget = 1),
            SpecError::Cache(LimitsError::BudgetBelowMin { budget: 1, min_budget: spec_min() }),
        ),
        ("shrink_to zero", Box::new(|spec| spec.cache.shrink_to = 0), SpecError::Cache(LimitsError::ShrinkToOutOfRange)),
        (
            "second video window inverted",
            Box::new(|spec| {
                let mut video = VideoSpec::new("other");
                video.prefetch = PrefetchSpec { from: 5, to: 2, depth: 2 };
                spec.videos.push(video);
            }),
            SpecError::Video { name: "other".to_string(), error: LimitsError::PrefetchWindowInverted { from: 5, to: 2 } },
        ),
        ("zero image size", Box::new(|spec| spec.images[0].size = Some((0, 288))), SpecError::ZeroImageSize { name: "avatar".to_string() }),
        (
            "image window inverted",
            Box::new(|spec| spec.images[0].prefetch_window = Some((3, 1))),
            SpecError::WindowInverted { name: "avatar".to_string() },
        ),
    ];
    for (label, change, expected) in cases {
        let mut spec = sample();
        change(&mut spec);
        assert_eq!(spec.validate(), Err(expected), "{label}");
    }
    assert_eq!(sample().validate(), Ok(()));
}

fn spec_min() -> u64 {
    CacheSpec::default().min
}

use super::*;
use crate::fetch::FetchPolicy;
use crate::limits::{EvictionStrategy, Limits, LimitsError};

fn sample() -> MediaSpec {
    let mut video = VideoSpec::new("moment_video");
    video.cover_field = Some("coverUrl".to_string());
    video.prefetch = PrefetchSpec { from: 1, to: 4, chunks: crate::limits::LeadingChunks::new(&[2, 2]).unwrap() };
    video.pool = PoolSpec { slots: 4, reach: 2, hold_off: 5, trim_delay: 60, low_ram_slots: Some(3), low_ram_reach: None };
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
            prefetch_behind: Some(2),
            evict: None,
        }],
        audios: vec![AudioSpec { name: "voice_note".to_string(), evict: Some(Evict::Fifo) }],
    }
}

#[test]
fn inv_spec_04_a_video_forwards_by_default_what_the_fetch_policy_forwards() {
    let policy_names: Vec<String> = crate::fetch::DEFAULT_FORWARDED_HEADERS.iter().map(|name| name.to_string()).collect();

    assert_eq!(VideoSpec::new("a").forward_headers, policy_names);
    assert_eq!(FetchPolicy::default().forwarded_headers, policy_names);
}

#[test]
fn inv_spec_02_json_is_stable_and_round_trips() {
    let spec = sample();
    let json = spec.to_json();
    assert_eq!(MediaSpec::from_json(&json).unwrap(), spec);
    assert_eq!(MediaSpec::from_json(&json).unwrap().to_json(), json);
    assert!(json.starts_with(r#"{"cache":{"budget":2147483648,"min":67108864,"shrink_to":80,"evict":{"expire_after":{"seconds":604800}}"#));
}

#[test]
fn inv_spec_02_json_with_an_unknown_option_is_rejected() {
    for text in [r#"{"cache":{"bogus":1}}"#, r#"{"videos":[{"name":"a","bogus":1}]}"#, r#"{"images":[{"name":"a","prefetch_window":[1,2,3]}]}"#] {
        assert!(MediaSpec::from_json(text).is_err(), "{text}");
    }
}

#[test]
fn inv_spec_04_limits_take_cache_values_and_the_first_video_and_default_to_core_limits() {
    assert_eq!(MediaSpec::default().limits(), Limits::default());
    let limits = sample().limits();
    assert_eq!(limits.budget, 2 * 1024 * 1024 * 1024);
    assert_eq!(limits.shrink_to, 0.8);
    assert_eq!(limits.strategy, EvictionStrategy::ExpireAfter(604800));
    assert_eq!((limits.slots, limits.prefetch_from, limits.prefetch_to, limits.leading_chunks.to_vec()), (4, 1, 4, vec![2, 2]));
    assert_eq!(limits.timeout_seconds, 30);
}

#[test]
fn inv_spec_03_validate_rejects_bad_specs() {
    type Change = Box<dyn Fn(&mut MediaSpec)>;
    let cases: Vec<(&str, Change, SpecError)> = vec![
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
                video.prefetch = PrefetchSpec { from: 5, to: 2, ..PrefetchSpec::default() };
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

#[test]
fn inv_spec_01_names_that_cannot_become_a_type_are_rejected_for_every_kind() {
    let bad = ["_", "", "__", "1clip", "a-b", "a b", "Avatar", "type", "self", "protocol", "any", "_clip", "clip_"];
    for name in bad {
        for kind in 0..3 {
            let mut spec = sample();
            spec.videos.clear();
            spec.images.clear();
            spec.audios.clear();
            match kind {
                0 => spec.videos.push(VideoSpec::new(name)),
                1 => spec.images.push(ImageSpec { name: name.to_string(), blurhash_field: None, size: None, prefetch_window: None, prefetch_behind: None, evict: None }),
                _ => spec.audios.push(AudioSpec { name: name.to_string(), evict: None }),
            }
            assert_eq!(spec.validate(), Err(SpecError::InvalidName(name.to_string())), "name {name:?} kind {kind}");
        }
    }
}

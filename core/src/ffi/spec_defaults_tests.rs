use super::*;
use crate::api::MediaError;
use crate::spec::{CacheSpec, Evict as SpecEvict, ImageSpec, MediaSpec, PoolSpec, PrefetchSpec, VideoSpec};

fn spec() -> MediaSpec {
    let mut video = VideoSpec::new("moment_video");
    video.prefetch = PrefetchSpec { from: 1, to: 4, chunks: crate::limits::LeadingChunks::new(&[4, 2]).unwrap() };
    video.pool = PoolSpec { slots: 4, reach: 2, hold_off: 5, trim_delay: 60, low_ram_slots: Some(3), low_ram_reach: Some(1) };
    let mut second = VideoSpec::new("chat_video");
    second.pool.slots = 9;
    MediaSpec {
        cache: CacheSpec { budget: 3_000_000_000, evict: SpecEvict::Fifo, namespace_headers: vec!["Authorization".to_string()], ..CacheSpec::default() },
        videos: vec![video, second],
        ..MediaSpec::default()
    }
}

#[test]
fn inv_spec_05_a_spec_json_becomes_the_defaults_the_pool_and_the_namespace_headers_of_open() {
    let parsed = spec_defaults(spec().to_json()).unwrap();

    assert_eq!((parsed.defaults.budget, parsed.defaults.slots, parsed.defaults.prefetch_to, parsed.defaults.evict), (3_000_000_000, 4, 4, Evict::Fifo));
    assert_eq!(parsed.pool, PoolSettings { reach: 2, hold_off_ms: 5_000, trim_delay_ms: 60_000, low_ram_slots: 3, low_ram_reach: 1 });
    assert_eq!(parsed.namespace_headers, vec!["Authorization".to_string()]);
    assert_eq!(spec_defaults("{}".to_string()).unwrap(), SpecDefaults { defaults: default_limits(), pool: default_pool_settings(), namespace_headers: Vec::new(), image_windows: Vec::new() });
}

#[test]
fn inv_spec_05_a_spec_json_that_does_not_parse_or_does_not_validate_is_refused() {
    let mut zero_budget = spec();
    zero_budget.cache.budget = 0;
    for (name, json) in [("not json", "{".to_string()), ("unknown option", r#"{"cache":{"bogus":1}}"#.to_string()), ("zero budget", zero_budget.to_json())] {
        assert!(matches!(spec_defaults(json), Err(MediaError::InvalidSpec { .. })), "{name}");
    }
}

#[test]
fn inv_spec_06_the_image_prefetch_window_and_behind_reach_open_per_image_and_an_image_without_them_has_none() {
    let image = |name: &str, window, behind| ImageSpec { name: name.to_string(), blurhash_field: None, size: None, prefetch_window: window, prefetch_behind: behind, evict: None };
    let spec = MediaSpec {
        images: vec![image("avatar", Some((1, 3)), Some(2)), image("poster", Some((0, 4)), None), image("grid", None, Some(3)), image("plain", None, None)],
        ..MediaSpec::default()
    };

    let windows = spec_defaults(spec.to_json()).unwrap().image_windows;

    let window = |name: &str, behind, from, to| ImageWindow { name: name.to_string(), behind, from, to };
    assert_eq!(windows, vec![window("avatar", 2, 1, 3), window("poster", 0, 0, 4), window("grid", 3, 0, 9)]);
}

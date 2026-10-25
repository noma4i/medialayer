use std::fs;
use std::path::PathBuf;

use medialayer::prefetch::CacheStrategy;

#[test]
fn inv_limits_01_dsl_configures_the_free_space_fraction() {
    let spec = crate::parse_spec("cache do\n  free_space 25.percent\nend\n").expect("valid disk share");
    assert!(spec.limits().encode().contains("free_space_fraction=0.25\n"));
}
use medialayer::spec::{AudioSpec, CacheSpec, Evict, ImageSpec, MediaSpec, PlaybackSpec, PoolSpec, PrefetchSpec, VideoSpec};

use super::*;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn ruby_blocks(path: &str) -> Vec<String> {
    let text = fs::read_to_string(root().join("..").join(path)).unwrap();
    text.split("```ruby\n").skip(1).map(|chunk| chunk.split("```").next().unwrap().to_string()).collect()
}

fn media_files_spec() -> MediaSpec {
    let mut video = VideoSpec::new("moment_video");
    video.cover_field = Some("coverUrl".into());
    video.prefetch = PrefetchSpec { from: 1, to: 3, chunks: medialayer::limits::LeadingChunks::new(&[2, 1]).unwrap() };
    video.pool = PoolSpec { slots: 3, reach: 1, hold_off: 1, trim_delay: 30, low_ram_slots: Some(2), low_ram_reach: Some(0) };
    video.forward_headers = vec!["Authorization".into(), "Cookie".into(), "User-Agent".into()];
    video.evict = Some(Evict::Lfu);
    video.strategy = CacheStrategy::Whole;
    MediaSpec {
        cache: CacheSpec {
            memory: 256 * 1024 * 1024,
            namespace_headers: vec!["Authorization".into()],
            ..CacheSpec::default()
        },
        videos: vec![video],
        images: vec![ImageSpec {
            name: "avatar".into(),
            blurhash_field: Some("blurHash".into()),
            size: Some((288, 288)),
            prefetch_window: Some((1, 3)),
            prefetch_behind: Some(2),
            evict: None,
            strategy: CacheStrategy::Window,
        }],
        audios: vec![AudioSpec { name: "voice_note".into(), evict: Some(Evict::Fifo), strategy: CacheStrategy::Window }],
    }
}

#[test]
fn the_media_files_reference_example_builds_the_documented_spec() {
    let blocks = ruby_blocks("docs/media-files.md");
    assert_eq!(blocks.len(), 1);
    assert_eq!(parse_spec(&blocks[0]).unwrap(), media_files_spec());
}

#[test]
fn every_ruby_example_in_the_docs_parses() {
    for path in ["README.md", "docs/memory.md", "docs/media-files.md", "docs/cache-strategies.md"] {
        for block in ruby_blocks(path) {
            assert!(parse_spec(&block).is_ok(), "{path}: {block}");
        }
    }
}

#[test]
fn expire_after_and_every_duration_unit_reach_the_spec() {
    let spec = parse_spec("cache do\n  evict expire_after 7.days\n  timeout 2.minutes\nend\nvideo :v do\n  pool hold_off: 1.hour\nend\n").unwrap();
    assert_eq!((spec.cache.evict, spec.cache.timeout, spec.videos[0].pool.hold_off), (Evict::ExpireAfter { seconds: 604800 }, 120, 3600));
}

#[test]
fn the_low_memory_pool_options_reach_the_spec_and_a_bad_value_names_its_place() {
    let spec = parse_spec("video :v do\n  pool low_ram_slots: 4, low_ram_reach: 1\nend\nvideo :w do\nend\n").unwrap();
    assert_eq!((spec.videos[0].pool.low_ram_slots, spec.videos[0].pool.low_ram_reach), (Some(4), Some(1)));
    assert_eq!((spec.videos[1].pool.low_ram_slots, spec.videos[1].pool.low_ram_reach), (None, None));

    let errors = parse_spec("video :v do\n  pool low_ram_slots: 1.second\nend\n").unwrap_err();
    assert_eq!(errors[0].message, "`low_ram_slots:` must be a whole number");
}

#[test]
fn several_files_merge_and_errors_name_their_file() {
    let directory = tempfile::tempdir().unwrap();
    let first = directory.path().join("a.mdl");
    let second = directory.path().join("b.mdl");
    fs::write(&first, "video :a do\nend\n").unwrap();
    fs::write(&second, "audio :b do\nend\n").unwrap();
    let spec = parse_files(&[&first, &second]).unwrap();
    assert_eq!((spec.videos.len(), spec.audios.len()), (1, 1));
    fs::write(&second, "audio :a do\nend\n").unwrap();
    let errors = parse_files(&[&first, &second]).unwrap_err();
    assert_eq!(errors[0].to_string(), format!("{}:1:7: `a` is declared twice", second.display()));
    assert_eq!(parse_files(&["/nonexistent/none.mdl"]).unwrap_err()[0].file, "/nonexistent/none.mdl");
}

#[test]
fn inv_readahead_05_a_video_declares_how_many_chunks_ahead_it_plays() {
    let spec = parse_spec("video :v do\n  playback chunks_ahead: 5\nend\nvideo :w do\nend\n").unwrap();
    assert_eq!((spec.videos[0].playback.chunks_ahead, spec.videos[1].playback.chunks_ahead), (5, 2));
    let errors = parse_spec("video :v do\n  playback chunks_ahead: 1.second\nend\n").unwrap_err();
    assert_eq!(errors[0].message, "`chunks_ahead:` must be a whole number");
}

#[test]
fn inv_strategy_14_every_kind_declares_its_cache_strategy_and_the_default_is_the_window() {
    let spec = parse_spec(
        "video :circle do\n  strategy :whole\nend\nimage :photo do\n  strategy :on_demand\nend\naudio :voice do\n  strategy :whole\nend\nvideo :feed do\nend\n",
    )
    .unwrap();
    assert_eq!(
        (spec.videos[0].strategy, spec.images[0].strategy, spec.audios[0].strategy, spec.videos[1].strategy),
        (CacheStrategy::Whole, CacheStrategy::OnDemand, CacheStrategy::Whole, CacheStrategy::Window)
    );
    let errors = parse_spec("video :v do\n  strategy :forever\nend\n").unwrap_err();
    assert_eq!(errors[0].message, "`strategy` must be one of :window, :whole or :on_demand");
}

#[test]
fn inv_strategy_14_generated_swift_and_kotlin_types_carry_the_strategy_with_the_same_name() {
    let spec = parse_spec("video :circle do\n  strategy :whole\nend\nimage :photo do\n  strategy :on_demand\nend\naudio :voice do\nend\n").unwrap();
    let (swift, kotlin) = (swift(&spec), kotlin(&spec, "app.media"));
    assert!(swift.contains("public var strategy: CacheStrategy { .whole }"), "{swift}");
    assert!(swift.contains("public var strategy: CacheStrategy { .onDemand }"), "{swift}");
    assert!(kotlin.contains("override val strategy: com.noma4i.medialayer.CacheStrategy get() = com.noma4i.medialayer.CacheStrategy.WHOLE"), "{kotlin}");
    assert!(kotlin.contains("CacheStrategy.ON_DEMAND"), "{kotlin}");
    assert_eq!(swift.matches("public var strategy").count(), 2);
    assert_eq!(kotlin.matches("override val strategy").count(), 2);
}

pub(crate) fn runtime_spec() -> MediaSpec {
    let mut video = VideoSpec::new("rich_video");
    video.cover_field = Some("coverUrl".into());
    video.prefetch = PrefetchSpec { from: 2, to: 5, chunks: medialayer::limits::LeadingChunks::new(&[2, 1]).unwrap() };
    video.pool = PoolSpec { slots: 4, reach: 2, hold_off: 7, trim_delay: 60, low_ram_slots: Some(1), low_ram_reach: Some(0) };
    video.playback = PlaybackSpec { chunks_ahead: 4 };
    video.forward_headers = vec!["Authorization".into(), "Cookie".into()];
    video.evict = Some(Evict::Lfu);
    video.strategy = CacheStrategy::Whole;
    MediaSpec {
        cache: CacheSpec {
            budget: 3 * 1024 * 1024 * 1024,
            min: 96 * 1024 * 1024,
            shrink_to: 75,
            evict: Evict::ExpireAfter { seconds: 2 * 86400 },
            memory: 192 * 1024 * 1024,
            timeout: 180,
            max_concurrent: 5,
            dedupe_window: 7200,
            dedupe_entries: 250,
            prefetch_budget_mib: 80,
            namespace_headers: vec!["Authorization".into(), "Cookie".into()],
            free_space: 15,
        },
        videos: vec![video, VideoSpec::new("plain_video")],
        images: vec![
            ImageSpec {
                name: "rich_image".into(),
                blurhash_field: Some("blurText".into()),
                size: Some((320, 240)),
                prefetch_window: Some((2, 6)),
                prefetch_behind: Some(3),
                evict: Some(Evict::LargestFirst),
                strategy: CacheStrategy::OnDemand,
            },
            ImageSpec {
                name: "plain_image".into(),
                blurhash_field: None,
                size: None,
                prefetch_window: None,
                prefetch_behind: None,
                evict: None,
                strategy: CacheStrategy::Window,
            },
            ImageSpec {
                name: "max_image".into(),
                blurhash_field: None,
                size: Some((i32::MAX as u32, i32::MAX as u32)),
                prefetch_window: None,
                prefetch_behind: None,
                evict: None,
                strategy: CacheStrategy::Window,
            },
        ],
        audios: vec![
            AudioSpec { name: "rich_audio".into(), evict: Some(Evict::Fifo), strategy: CacheStrategy::Whole },
            AudioSpec { name: "plain_audio".into(), evict: None, strategy: CacheStrategy::Window },
        ],
    }
}

#[test]
fn inv_gen_03_the_runtime_fixture_builds_every_option_with_the_contract_value() {
    let parsed = parse_files(&[root().join("fixtures/runtime.mdl")]).unwrap();
    assert_eq!(parsed, runtime_spec());
}

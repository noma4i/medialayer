use crate::parse_spec;

fn failure(source: &str) -> (usize, usize, String) {
    let errors = parse_spec(source).unwrap_err();
    assert_eq!(errors.len(), 1, "{errors:?}");
    (errors[0].line, errors[0].column, errors[0].message.clone())
}

fn expected(line: usize, column: usize, message: &str) -> (usize, usize, String) {
    (line, column, message.to_string())
}

#[test]
fn invalid_sources_are_reported_with_line_column_and_message() {
    let cases = [
        ("frob :x do\nend\n", expected(1, 1, "unknown block `frob`, use cache, video, image or audio")),
        ("video :a do\n  bogus 1\nend\n", expected(2, 3, "unknown option `bogus` for `video`")),
        ("video :a do\n  prefetch frm: 1\nend\n", expected(2, 12, "unknown key `frm` for `prefetch`")),
        ("cache do\n  budget 4.seconds\nend\n", expected(2, 10, "`budget` must be a size such as 64.mib or 4.gib")),
        ("cache do\n  budget 4.gib, min: 5\nend\n", expected(2, 22, "`min:` must be a size such as 64.mib or 4.gib")),
        ("cache do\n  evict :random\nend\n", expected(2, 9, "`evict` must be one of :lru, :fifo, :lfu, :largest_first or expire_after <duration>")),
        ("image :a do\n  size 288\nend\n", expected(2, 3, "`size` takes 2 value(s), found 1")),
        ("image :a do\n  prefetch window: 3\nend\n", expected(2, 20, "`window:` must be a range such as 1...3")),
        ("cache do\n  dedupe\nend\n", expected(2, 3, "`dedupe` needs at least one option")),
        ("cache do\n  memory 1.gib\n  memory 2.gib\nend\n", expected(3, 3, "`memory` is set twice")),
        ("cache do\nend\ncache do\nend\n", expected(3, 1, "`cache` is declared twice")),
        ("audio :a do\nend\nvideo :a do\nend\n", expected(3, 7, "`a` is declared twice")),
        ("video do\nend\n", expected(1, 1, "`video` needs a symbol name, for example `video :name do`")),
        ("audio :a\n", expected(1, 1, "`audio` needs a `do ... end` block")),
        ("cache do\n  budget 1.mib\nend\n", expected(1, 1, "cache: budget 1048576 is below min_budget 67108864")),
        ("cache do\n  shrink_to 0.percent\nend\n", expected(1, 1, "cache: shrink_to must be in (0, 1]")),
        ("video :a do\n  prefetch from: 5, to: 2\nend\n", expected(1, 1, "video `a`: prefetch window from 5 is above to 2")),
        ("video :a do\n  prefetch chunks: [1, 2]\nend\n", expected(2, 20, "`chunks:` must be a list of up to 8 chunk counts, each 1 or 2 (the owner cap), that never grow, such as [2, 1]")),
        ("cache do\n  prefetch_budget 600.mib\nend\n", expected(1, 1, "cache: prefetch_budget_mib must be between 1 and 512")),
        ("image :a do\n  size 0, 288\nend\n", expected(1, 1, "`a` has a zero width or height")),
        (
            "video :_ do\nend\n",
            expected(1, 1, "`_` cannot become a type name: use lowercase letters, digits and single underscores, starting with a letter"),
        ),
    ];
    for (source, want) in cases {
        assert_eq!(failure(source), want, "{source}");
    }
}

#[test]
fn every_error_of_a_file_is_reported_at_once() {
    let errors = parse_spec("video :a do\n  bogus 1\n  other 2\nend\n").unwrap_err();
    assert_eq!(errors.iter().map(|error| error.line).collect::<Vec<_>>(), vec![2, 3]);
}

#[test]
fn inv_gen_01_options_are_unique_and_numbers_reach_their_destination_without_truncation() {
    let cases = [
        "video :clip do\n  pool slots: 3, slots: 5\nend",
        "video :clip do\n  prefetch from: 1, from: 2\nend",
        "image :photo do\n  prefetch behind: 1, behind: 2\nend",
        "cache do\n  budget 4.gib, min: 1.mib, min: 2.mib\nend",
        "cache do\n  dedupe entries: 1, entries: 2\nend",
        "cache do\n  namespace headers: [:cookie], headers: [:authorization]\nend",
        "video :clip, ignored: 1 do\nend",
        "image :photo, ignored: 1 do\nend",
        "audio :voice, ignored: 1 do\nend",
        "cache ignored: 1 do\nend",
        "cache do\n  prefetch_budget 4294967297.mib\nend",
        "cache do\n  prefetch_budget 4194304.gib\nend",
    ];
    let accepted: Vec<_> = cases.into_iter().filter(|source| parse_spec(source).is_ok()).collect();
    assert!(accepted.is_empty(), "invalid inputs accepted: {accepted:#?}");
    for amount in [1, 512] {
        let spec = parse_spec(&format!("cache do\n  prefetch_budget {amount}.mib\nend")).unwrap();
        assert_eq!(spec.cache.prefetch_budget_mib, amount);
    }
}

#[test]
fn inv_gen_02_names_and_dimensions_are_valid_for_both_generated_languages() {
    let names = [
        "media_spec_json", "media_spec_j_s_o_n", "media_source", "media_size", "kind", "cache_strategy", "sendable",
        "string", "int", "pair", "map",
    ];
    let mut accepted = Vec::new();
    for kind in ["video", "image", "audio"] {
        for name in names {
            let source = format!("{kind} :{name} do\nend");
            if parse_spec(&source).is_ok() {
                accepted.push(source);
            }
        }
    }
    for dimensions in ["2147483648, 1", "1, 2147483648", "4294967295, 4294967295"] {
        let source = format!("image :photo do\n  size {dimensions}\nend");
        if parse_spec(&source).is_ok() {
            accepted.push(source);
        }
    }
    assert!(accepted.is_empty(), "invalid inputs accepted: {accepted:#?}");
    let spec = parse_spec("image :photo do\n  size 2147483647, 2147483647\nend").unwrap();
    assert_eq!(spec.images[0].size, Some((i32::MAX as u32, i32::MAX as u32)));
}

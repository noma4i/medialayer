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
        ("image :a do\n  size 0, 288\nend\n", expected(1, 1, "`a` has a zero width or height")),
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

use super::*;
use crate::lexer::lex;

fn parsed(source: &str) -> Vec<Call> {
    parse(&lex(source).unwrap()).unwrap()
}

#[test]
fn blocks_arguments_options_ranges_arrays_and_calls_nest() {
    let calls = parsed("image :avatar do\n  size 288, 288\n  prefetch window: 1...3\n  evict expire_after 7.days\n  forward_headers [:a, :b]\nend\n");
    let body = calls[0].block.as_ref().unwrap();
    assert_eq!((calls[0].word.as_str(), body.len()), ("image", 4));
    assert_eq!(calls[0].args[0].item, Value::Symbol("avatar".into()));
    assert_eq!(body[0].args.iter().map(|arg| arg.item.clone()).collect::<Vec<_>>(), vec![Value::Int(288), Value::Int(288)]);
    assert_eq!(body[1].options[0].value.item, Value::Range { from: 1, to: 3 });
    assert!(matches!(&body[2].args[0].item, Value::Call { word, .. } if word == "expire_after"));
    assert!(matches!(&body[3].args[0].item, Value::Array(items) if items.len() == 2));
}

#[test]
fn malformed_source_is_reported_with_its_position() {
    let cases = [
        ("cache do\n  evict :lru\n", ParseError::new(1, 1, "`cache` block is not closed with `end`")),
        ("cache do budget 1\nend", ParseError::new(1, 10, "unexpected `budget`")),
        ("cache do\n  budget 1,\nend\n", ParseError::new(3, 1, "unexpected `end`")),
        ("1 :x", ParseError::new(1, 1, "unexpected `1`")),
        ("a :x do\n  prefetch window: 1...\nend", ParseError::new(2, 24, "unexpected end of line")),
    ];
    for (source, expected) in cases {
        let error = parse(&lex(source).unwrap()).unwrap_err();
        assert_eq!(error, expected, "{source}");
    }
}

use super::*;

fn kinds(source: &str) -> Vec<Kind> {
    lex(source).unwrap().into_iter().map(|token| token.kind).collect()
}

fn quantity(amount: u64, unit: &str) -> Kind {
    Kind::Quantity { amount, unit: unit.to_string() }
}

#[test]
fn every_literal_form_of_the_language() {
    assert_eq!(
        kinds("budget 4.gib, min: 64.mib # note\nprefetch window: 1...3\nforward_headers [:user_agent]"),
        vec![
            Kind::Word("budget".into()),
            quantity(4, "gib"),
            Kind::Comma,
            Kind::Word("min".into()),
            Kind::Colon,
            quantity(64, "mib"),
            Kind::Newline,
            Kind::Word("prefetch".into()),
            Kind::Word("window".into()),
            Kind::Colon,
            Kind::Int(1),
            Kind::Range,
            Kind::Int(3),
            Kind::Newline,
            Kind::Word("forward_headers".into()),
            Kind::LeftBracket,
            Kind::Symbol("user_agent".into()),
            Kind::RightBracket,
        ]
    );
}

#[test]
fn every_documented_unit_lexes() {
    for unit in UNITS {
        assert_eq!(kinds(&format!("5.{unit}")), vec![quantity(5, unit)], "{unit}");
    }
}

#[test]
fn a_comma_continues_the_line() {
    assert_eq!(kinds("a 1,\n  b: 2").iter().filter(|kind| **kind == Kind::Newline).count(), 0);
}

#[test]
fn bad_input_is_reported_with_its_position() {
    let cases = [("a\n  4.weeks", (2, 3, "unknown unit `weeks`")), ("x @", (1, 3, "unexpected character `@`"))];
    for (source, (line, column, message)) in cases {
        assert_eq!(lex(source).unwrap_err(), ParseError::new(line, column, message), "{source}");
    }
}

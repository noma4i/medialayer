use std::collections::{HashMap, HashSet};

use medialayer::spec::{type_name, SpecError};

use crate::build::{self, build};
use crate::parser::{Call, Node, Value};
use crate::ParseError;

type Found = (usize, ParseError);

#[derive(Clone, Copy)]
enum Want {
    Symbol,
    Int,
    Size,
    Duration,
    Percent,
    Evict,
    Headers,
    Range,
    Chunks,
}

impl Want {
    fn describe(self) -> &'static str {
        match self {
            Want::Symbol => "a symbol such as :name",
            Want::Int => "a whole number",
            Want::Size => "a size such as 64.mib or 4.gib",
            Want::Duration => "a duration such as 30.seconds",
            Want::Percent => "a percentage such as 90.percent",
            Want::Evict => "one of :lru, :fifo, :lfu, :largest_first or expire_after <duration>",
            Want::Headers => "a list of symbols such as [:authorization]",
            Want::Range => "a range such as 1...3",
            Want::Chunks => "a list of 1 to 8 chunk counts between 1 and 8 that never grow, such as [3, 2, 1]",
        }
    }

    fn accepts(self, node: &Node<Value>) -> bool {
        match self {
            Want::Symbol => build::symbol(node).is_some(),
            Want::Int => build::int(node).is_some(),
            Want::Size => build::size(node).is_some(),
            Want::Duration => build::duration(node).is_some(),
            Want::Percent => build::percent(node).is_some(),
            Want::Evict => build::evict(node).is_some(),
            Want::Headers => build::headers(node).is_some(),
            Want::Range => build::range(node).is_some(),
            Want::Chunks => build::chunks(node).is_some(),
        }
    }
}

struct Rule {
    word: &'static str,
    args: &'static [Want],
    keys: &'static [(&'static str, Want)],
}

const EVICT: Rule = Rule { word: "evict", args: &[Want::Evict], keys: &[] };

const CACHE: &[Rule] = &[
    Rule { word: "budget", args: &[Want::Size], keys: &[("min", Want::Size)] },
    Rule { word: "shrink_to", args: &[Want::Percent], keys: &[] },
    EVICT,
    Rule { word: "memory", args: &[Want::Size], keys: &[] },
    Rule { word: "timeout", args: &[Want::Duration], keys: &[] },
    Rule { word: "max_concurrent", args: &[Want::Int], keys: &[] },
    Rule { word: "dedupe", args: &[], keys: &[("window", Want::Duration), ("entries", Want::Int)] },
    Rule { word: "prefetch_budget", args: &[Want::Size], keys: &[] },
    Rule { word: "namespace", args: &[], keys: &[("headers", Want::Headers)] },
];

const VIDEO: &[Rule] = &[
    Rule { word: "cover", args: &[Want::Symbol], keys: &[] },
    Rule { word: "prefetch", args: &[], keys: &[("from", Want::Int), ("to", Want::Int), ("chunks", Want::Chunks)] },
    Rule {
        word: "pool",
        args: &[],
        keys: &[
            ("slots", Want::Int),
            ("reach", Want::Int),
            ("hold_off", Want::Duration),
            ("trim_delay", Want::Duration),
            ("low_ram_slots", Want::Int),
            ("low_ram_reach", Want::Int),
        ],
    },
    Rule { word: "forward_headers", args: &[Want::Headers], keys: &[] },
    EVICT,
];

const IMAGE: &[Rule] = &[
    Rule { word: "blurhash", args: &[Want::Symbol], keys: &[] },
    Rule { word: "size", args: &[Want::Int, Want::Int], keys: &[] },
    Rule { word: "prefetch", args: &[], keys: &[("window", Want::Range), ("behind", Want::Int)] },
    EVICT,
];

const AUDIO: &[Rule] = &[EVICT];

pub fn check(files: &[Vec<Call>]) -> Result<medialayer::spec::MediaSpec, Vec<Found>> {
    let mut errors = Vec::new();
    let mut cache_seen = false;
    let mut type_names = HashSet::new();
    for (file, calls) in files.iter().enumerate() {
        for call in calls {
            let mut fail = |line: usize, column: usize, message: String| errors.push((file, ParseError::new(line, column, message)));
            let rules: &[Rule] = match call.word.as_str() {
                "cache" => {
                    if std::mem::replace(&mut cache_seen, true) {
                        fail(call.line, call.column, "`cache` is declared twice".to_string());
                    }
                    CACHE
                }
                "video" => VIDEO,
                "image" => IMAGE,
                "audio" => AUDIO,
                other => {
                    fail(call.line, call.column, format!("unknown block `{other}`, use cache, video, image or audio"));
                    continue;
                }
            };
            if call.word == "cache" {
                if let Some(extra) = call.args.first() {
                    fail(extra.line, extra.column, "`cache` takes no name".to_string());
                }
            } else {
                match call.args.as_slice() {
                    [Node { item: Value::Symbol(name), line, column }] => {
                        if !type_names.insert(type_name(name)) {
                            fail(*line, *column, format!("`{name}` is declared twice"));
                        }
                    }
                    _ => fail(call.line, call.column, format!("`{}` needs a symbol name, for example `{} :name do`", call.word, call.word)),
                }
            }
            let Some(body) = &call.block else {
                fail(call.line, call.column, format!("`{}` needs a `do ... end` block", call.word));
                continue;
            };
            let mut seen = HashSet::new();
            for option in body {
                for (line, column, message) in check_option(option, rules, &call.word, &mut seen) {
                    fail(line, column, message);
                }
            }
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let built = build(files);
    built.spec.validate().map_err(|error| vec![locate(&error, &built)])?;
    Ok(built.spec)
}

fn locate(error: &SpecError, built: &build::Built) -> Found {
    let named = match error {
        SpecError::DuplicateName(name)
        | SpecError::InvalidName(name)
        | SpecError::ZeroImageSize { name }
        | SpecError::WindowInverted { name }
        | SpecError::Video { name, .. } => built.blocks.get(name),
        SpecError::Cache(_) => built.cache_at.as_ref(),
    };
    let (file, line, column) = named.copied().unwrap_or((0, 1, 1));
    (file, ParseError::new(line, column, error.to_string()))
}

fn check_option(call: &Call, rules: &[Rule], block: &str, seen: &mut HashSet<String>) -> Vec<(usize, usize, String)> {
    let Some(rule) = rules.iter().find(|rule| rule.word == call.word) else {
        return vec![(call.line, call.column, format!("unknown option `{}` for `{block}`", call.word))];
    };
    let mut errors = Vec::new();
    if !seen.insert(call.word.clone()) {
        errors.push((call.line, call.column, format!("`{}` is set twice", call.word)));
    }
    if call.block.is_some() {
        errors.push((call.line, call.column, format!("`{}` takes no block", call.word)));
    }
    if call.args.len() != rule.args.len() {
        errors.push((call.line, call.column, format!("`{}` takes {} value(s), found {}", call.word, rule.args.len(), call.args.len())));
    }
    for (node, want) in call.args.iter().zip(rule.args) {
        if !want.accepts(node) {
            errors.push((node.line, node.column, format!("`{}` must be {}", call.word, want.describe())));
        }
    }
    let known: HashMap<&str, Want> = rule.keys.iter().copied().collect();
    for pair in &call.options {
        match known.get(pair.key.as_str()) {
            None => errors.push((pair.line, pair.column, format!("unknown key `{}` for `{}`", pair.key, call.word))),
            Some(want) if !want.accepts(&pair.value) => {
                errors.push((pair.value.line, pair.value.column, format!("`{}:` must be {}", pair.key, want.describe())))
            }
            Some(_) => {}
        }
    }
    if rule.args.is_empty() && call.options.is_empty() {
        errors.push((call.line, call.column, format!("`{}` needs at least one option", call.word)));
    }
    errors
}

#[cfg(test)]
mod check_tests;

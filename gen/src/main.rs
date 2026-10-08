use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use medialayer_gen::{kotlin, parse_files, swift};

const USAGE: &str = "usage: medialayer-gen spec <file.mdl>...\n       medialayer-gen swift <file.mdl>... --out <dir>\n       medialayer-gen kotlin <file.mdl>... --out <dir> --package <pkg>";

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let Some((command, rest)) = arguments.split_first() else {
        return usage();
    };
    let (files, options) = split_options(rest);
    if files.is_empty() {
        return usage();
    }
    let spec = match parse_files(&files) {
        Ok(spec) => spec,
        Err(errors) => {
            errors.iter().for_each(|error| eprintln!("{error}"));
            return ExitCode::from(1);
        }
    };
    let option = |name: &str| options.iter().find(|(key, _)| key == name).map(|(_, value)| value.clone());
    match (command.as_str(), option("--out"), option("--package")) {
        ("spec", None, None) => {
            println!("{}", serde_json::to_string_pretty(&spec).expect("spec serializes"));
            ExitCode::SUCCESS
        }
        ("swift", Some(out), None) => write(&out, "MediaSources.swift", &swift(&spec)),
        ("kotlin", Some(out), Some(package)) => write(&out, "MediaSources.kt", &kotlin(&spec, &package)),
        _ => usage(),
    }
}

fn split_options(arguments: &[String]) -> (Vec<String>, Vec<(String, String)>) {
    let mut files = Vec::new();
    let mut options = Vec::new();
    let mut iterator = arguments.iter();
    while let Some(argument) = iterator.next() {
        if argument.starts_with("--") {
            options.push((argument.clone(), iterator.next().cloned().unwrap_or_default()));
        } else {
            files.push(argument.clone());
        }
    }
    (files, options)
}

fn write(directory: &str, name: &str, text: &str) -> ExitCode {
    let path = PathBuf::from(directory).join(name);
    match fs::create_dir_all(directory).and_then(|_| fs::write(&path, text)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{}: {error}", path.display());
            ExitCode::from(1)
        }
    }
}

fn usage() -> ExitCode {
    eprintln!("{USAGE}");
    ExitCode::from(2)
}

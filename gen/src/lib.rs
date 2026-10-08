mod build;
mod check;
mod kotlin;
mod lexer;
mod parser;
mod source;
mod swift;

use std::fmt;
use std::fs;
use std::path::Path;

pub use kotlin::kotlin;
use medialayer::spec::MediaSpec;
pub use swift::swift;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub line: usize,
    pub column: usize,
    pub message: String,
}

impl ParseError {
    pub fn new(line: usize, column: usize, message: impl Into<String>) -> Self {
        Self { line, column, message: message.into() }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}:{}: {}", self.line, self.column, self.message)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileError {
    pub file: String,
    pub error: ParseError,
}

impl fmt::Display for FileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}:{}:{}: {}", self.file, self.error.line, self.error.column, self.error.message)
    }
}

pub fn parse_spec(source: &str) -> Result<MediaSpec, Vec<ParseError>> {
    parse_sources(&[source]).map_err(|errors| errors.into_iter().map(|(_, error)| error).collect())
}

pub fn parse_files(paths: &[impl AsRef<Path>]) -> Result<MediaSpec, Vec<FileError>> {
    let names: Vec<String> = paths.iter().map(|path| path.as_ref().display().to_string()).collect();
    let mut sources = Vec::new();
    let mut errors = Vec::new();
    for (path, name) in paths.iter().zip(&names) {
        match fs::read_to_string(path.as_ref()) {
            Ok(text) => sources.push(text),
            Err(error) => errors.push(FileError { file: name.clone(), error: ParseError::new(0, 0, error.to_string()) }),
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let sources: Vec<&str> = sources.iter().map(String::as_str).collect();
    parse_sources(&sources).map_err(|found| {
        found.into_iter().map(|(file, error)| FileError { file: names[file].clone(), error }).collect()
    })
}

fn parse_sources(sources: &[&str]) -> Result<MediaSpec, Vec<(usize, ParseError)>> {
    let mut files = Vec::new();
    let mut errors = Vec::new();
    for (file, source) in sources.iter().enumerate() {
        match lexer::lex(source).and_then(|tokens| parser::parse(&tokens)) {
            Ok(calls) => files.push(calls),
            Err(error) => errors.push((file, error)),
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    check::check(&files)
}

#[cfg(test)]
mod lib_tests;

use crate::identity::{hex_lower, key_file_name};
use std::path::{Path, PathBuf};

pub const INDEX_FILE: &str = "index.tsv";
pub const INDEX_TMP_FILE: &str = "index.tsv.tmp";
pub const BIN_SUFFIX: &str = ".bin";
pub const PART_SUFFIX: &str = ".part";

pub fn hex_encode(text: &str) -> String {
    hex_lower(text.as_bytes())
}

pub fn hex_decode(hex: &str) -> Option<String> {
    if !hex.len().is_multiple_of(2) || !hex.is_ascii() {
        return None;
    }
    let bytes: Option<Vec<u8>> = (0..hex.len()).step_by(2).map(|at| u8::from_str_radix(&hex[at..at + 2], 16).ok()).collect();
    String::from_utf8(bytes?).ok()
}

pub fn file_name(key: &str) -> String {
    key_file_name(key)
}

pub fn bin_path(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}{BIN_SUFFIX}"))
}

pub fn part_path(dir: &Path, name: &str, sequence: u64) -> PathBuf {
    dir.join(format!("{name}.{sequence}{PART_SUFFIX}"))
}

pub fn name_of_bin_file(file: &str) -> Option<String> {
    file.strip_suffix(BIN_SUFFIX).map(str::to_string)
}

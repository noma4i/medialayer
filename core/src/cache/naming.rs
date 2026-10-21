use crate::identity::key_file_name;
use std::path::{Path, PathBuf};

pub const INDEX_FILE: &str = "index.tsv";
pub const INDEX_TMP_FILE: &str = "index.tsv.tmp";
pub const BIN_SUFFIX: &str = ".bin";
pub const PART_SUFFIX: &str = ".part";

#[cfg(test)]
pub fn hex_encode(text: &str) -> String {
    crate::identity::hex_lower(text.as_bytes())
}

pub fn hex_decode_bytes(hex: &str) -> Option<Vec<u8>> {
    if !hex.len().is_multiple_of(2) || !hex.is_ascii() {
        return None;
    }
    (0..hex.len()).step_by(2).map(|at| u8::from_str_radix(&hex[at..at + 2], 16).ok()).collect()
}

pub fn hex_decode(hex: &str) -> Option<String> {
    String::from_utf8(hex_decode_bytes(hex)?).ok()
}

pub fn file_name(key: &str) -> String {
    #[cfg(test)]
    NAME_HASHES.with(|count| count.set(count.get() + 1));
    key_file_name(key)
}

#[cfg(test)]
thread_local! {
    pub static NAME_HASHES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
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

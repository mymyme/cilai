//! Read-path helpers for Codex memories.
//!
//! This crate owns memory injection, memory citation parsing, and telemetry
//! classification for read access to the memory folder. It intentionally does
//! not depend on the memory write pipeline.

pub mod citations;
mod metrics;
pub mod usage;

use codex_utils_absolute_path::AbsolutePathBuf;

pub fn memory_root(codex_home: &AbsolutePathBuf) -> AbsolutePathBuf {
    let publication = codex_home.join("memory-publication");
    if !publication.is_dir() {
        return codex_home.join("memories");
    }
    let revision = std::fs::read_to_string(publication.join("current"))
        .ok()
        .filter(|value| {
            value.len() == 36 && value.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
        });
    match revision {
        Some(revision) => publication.join("versions").join(revision).join("memories"),
        None => publication.join("unpublished").join("memories"),
    }
}

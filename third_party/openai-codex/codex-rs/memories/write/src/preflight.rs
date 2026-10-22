use serde_json::json;
use std::path::Path;
use tokio::fs;

pub(crate) async fn describe(root: &Path) -> anyhow::Result<String> {
    let mut files = Vec::new();
    for name in ["MEMORY.md", "memory_summary.md", "raw_memories.md"] {
        let content = match fs::read_to_string(root.join(name)).await {
            Ok(content) => Some(content),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        files.push(json!({
            "path": name,
            "exists": content.is_some(),
            "bytes": content.as_ref().map(String::len),
            "lines": content.as_ref().map(|text| text.lines().count()),
            "summaryVersionValid": (name == "memory_summary.md")
                .then(|| content.as_ref().is_some_and(|text| text.lines().next() == Some("v1"))),
        }));
    }
    Ok(format!(
        "Verified file metadata for this run (do not repeat existence, size, or line-count checks): {}. Missing optional skills do not select INIT mode. If MEMORY.md already has content, reconcile it with changed sources and rebuild a missing or incompatible summary; do not discard the index or restart a full INIT exploration merely because the summary is missing. Existing drafts still require evidence checks before completion. Choose non-overlapping read ranges from these sizes, batch independent calls in the same model response, and do not reread content already visible unless a tool explicitly truncated it. Read sources as UTF-8 and use the decoded text directly. Do not probe shell versions, character codes or byte encodings unless a file operation actually fails. On Windows PowerShell 5, Set-Content -Encoding UTF8 adds a BOM; write the required files with UTF-8 without BOM so the summary starts with the literal v1 line. Prioritize completing a missing summary once the existing index has been checked. After source coverage, finish both required files and return for host validation. No optional artifacts are part of this run.",
        serde_json::to_string(&files)?
    ))
}

#[cfg(test)]
#[path = "preflight_tests.rs"]
mod tests;

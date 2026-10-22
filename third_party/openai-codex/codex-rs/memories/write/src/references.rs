use std::collections::BTreeSet;
use std::path::Component;
use std::path::Path;
use tokio::fs;

pub(crate) async fn validate_rollout_references(root: &Path, texts: &[&str]) -> anyhow::Result<()> {
    let mut references = BTreeSet::new();
    for text in texts {
        for (offset, _) in text.match_indices("rollout_summaries/") {
            let remaining = &text[offset..];
            let end = remaining
                .find(|ch: char| ch.is_whitespace() || "`\"'<>[]()".contains(ch))
                .unwrap_or(remaining.len());
            let candidate = &remaining[..end];
            let Some(extension) = candidate.find(".md") else {
                continue;
            };
            let reference = &candidate[..extension + 3];
            let suffix = &candidate[extension + 3..];
            anyhow::ensure!(
                !suffix.contains('/') && !suffix.contains('\\'),
                "invalid memory rollout reference path: {candidate}"
            );
            if reference.contains('*') {
                continue;
            }
            anyhow::ensure!(
                Path::new(reference)
                    .components()
                    .all(|part| matches!(part, Component::Normal(_))),
                "memory rollout reference escapes the source directory: {reference}"
            );
            references.insert(reference.to_owned());
        }
    }
    for text in texts {
        let mut in_sources = false;
        for line in text.lines() {
            if line.starts_with('#') {
                in_sources = line.trim() == "### rollout_summary_files";
                continue;
            }
            if !in_sources || line.contains("rollout_summaries/") {
                continue;
            }
            let Some(bullet) = line.trim_start().strip_prefix("- ") else {
                continue;
            };
            let candidate = bullet.trim_start_matches(['`', '<']);
            let Some(extension) = candidate.find(".md") else {
                anyhow::bail!("memory source entry does not identify a rollout file");
            };
            let name = &candidate[..extension + 3];
            anyhow::ensure!(
                !name.chars().any(char::is_whitespace)
                    && !name.contains('/')
                    && !name.contains('\\'),
                "invalid memory source filename: {name}"
            );
            references.insert(format!("rollout_summaries/{name}"));
        }
    }
    for reference in references {
        let path = root.join(&reference);
        let metadata = fs::symlink_metadata(&path).await.map_err(|error| {
            anyhow::anyhow!("memory rollout reference is unavailable: {reference}: {error}")
        })?;
        anyhow::ensure!(
            metadata.is_file() && !metadata.is_symlink(),
            "memory rollout reference is not a regular source file: {reference}"
        );
    }
    Ok(())
}

#[cfg(test)]
#[path = "references_tests.rs"]
mod tests;

use anyhow::Context;
use serde_json::json;
use std::path::Path;
use tokio::fs;

pub(crate) async fn last_error(home: &Path) -> anyhow::Result<Option<String>> {
    let bytes = match fs::read(home.join("memory-publication/status.json")).await {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let value: serde_json::Value = serde_json::from_slice(&bytes)?;
    Ok(value["error"].as_str().map(str::to_owned))
}

pub(crate) async fn status(home: &Path, phase: &str, error: Option<&str>) -> anyhow::Result<()> {
    let directory = home.join("memory-publication");
    if !directory.is_dir() {
        return Ok(());
    }
    let value = json!({"phase":phase,"updatedAt":chrono::Utc::now().timestamp(),"error":error});
    atomic_write(&directory, "status.json", &serde_json::to_vec(&value)?).await
}

pub(crate) async fn atomic_write(directory: &Path, name: &str, bytes: &[u8]) -> anyhow::Result<()> {
    let temporary = directory.join(format!(".{name}-{}", uuid::Uuid::new_v4()));
    fs::write(&temporary, bytes).await?;
    fs::rename(&temporary, directory.join(name)).await?;
    Ok(())
}

pub(crate) async fn extension_sources(root: &Path) -> anyhow::Result<usize> {
    let mut count = 0;
    let mut directories = vec![root.join("extensions")];
    while let Some(directory) = directories.pop() {
        let mut entries = match fs::read_dir(directory).await {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        while let Some(entry) = entries.next_entry().await? {
            let kind = entry.file_type().await?;
            anyhow::ensure!(
                !kind.is_symlink(),
                "memory extension contains a symbolic link"
            );
            if kind.is_dir() {
                directories.push(entry.path());
            } else if kind.is_file()
                && entry.file_name() != "instructions.md"
                && entry.metadata().await?.len() > 0
            {
                count += 1;
            }
        }
    }
    Ok(count)
}

pub(crate) async fn publish(root: &Path, source_count: usize) -> anyhow::Result<()> {
    let directory = root
        .parent()
        .context("missing memory home")?
        .join("memory-publication");
    if !directory.is_dir() {
        return Ok(());
    }
    if source_count == 0 {
        atomic_write(&directory, "current", b"").await?;
        return Ok(());
    }
    crate::workspace::validate_consolidation_artifacts(root).await?;
    anyhow::ensure!(
        !fs::read_to_string(root.join("MEMORY.md"))
            .await?
            .trim()
            .is_empty(),
        "empty memory index"
    );
    let summary = fs::read_to_string(root.join("memory_summary.md")).await?;
    anyhow::ensure!(
        summary.lines().skip(1).any(|line| !line.trim().is_empty()),
        "empty memory summary"
    );
    let revision = uuid::Uuid::new_v4().to_string();
    let snapshot = directory.join("versions").join(&revision);
    let output = snapshot.join("memories");
    fs::create_dir_all(&output).await?;
    let result = async {
    let mut directories = vec![(root.to_path_buf(), output.clone())];
    let mut total_bytes = 0_u64;
    while let Some((source, target)) = directories.pop() {
        let mut entries = fs::read_dir(&source).await?;
        while let Some(entry) = entries.next_entry().await? {
            let name = entry.file_name();
            if name.to_string_lossy().starts_with('.') || name == "phase2_workspace_diff.md" {
                continue;
            }
            let kind = entry.file_type().await?;
            anyhow::ensure!(
                !kind.is_symlink(),
                "memory publication contains a symbolic link"
            );
            let destination = target.join(&name);
            if kind.is_dir() {
                fs::create_dir_all(&destination).await?;
                directories.push((entry.path(), destination));
            } else if kind.is_file() {
                total_bytes += entry.metadata().await?.len();
                anyhow::ensure!(
                    total_bytes <= 16 * 1024 * 1024,
                    "memory publication exceeds size budget"
                );
                let bytes = fs::read(entry.path()).await?;
                if entry
                    .path()
                    .extension()
                    .is_some_and(|extension| extension == "md")
                {
                    let text = String::from_utf8(bytes)?;
                    let text = text.replace(
                        &format!("{}/", root.display()),
                        &format!("{}/", output.display()),
                    );
                    fs::write(destination, text).await?;
                } else {
                    fs::write(destination, bytes).await?;
                }
            }
        }
    }
    if let Ok(previous_revision) = fs::read_to_string(directory.join("current")).await
        && uuid::Uuid::parse_str(&previous_revision).is_ok()
    {
        let previous_snapshot = directory.join("versions").join(previous_revision);
        let previous = previous_snapshot.join("memories");
        let manifest = fs::read(previous_snapshot.join("manifest.json")).await?;
        let manifest: serde_json::Value = serde_json::from_slice(&manifest)?;
        if manifest["sourceCount"].as_u64() == Some(source_count as u64)
            && snapshot_files(&previous).await? == snapshot_files(&output).await?
        {
            fs::remove_dir_all(&snapshot).await?;
            return Ok(());
        }
    }
    fs::write(snapshot.join("manifest.json"), serde_json::to_vec(&json!({"revision":revision,"sourceCount":source_count,"publishedAt":chrono::Utc::now().timestamp()}))?).await?;
    atomic_write(&directory, "current", revision.as_bytes()).await
    }.await;
    if result.is_err()
        && let Err(error) = fs::remove_dir_all(&snapshot).await
    {
        tracing::warn!(%error, "failed removing incomplete memory snapshot");
    }
    result
}

async fn snapshot_files(
    root: &Path,
) -> anyhow::Result<std::collections::BTreeMap<std::path::PathBuf, Vec<u8>>> {
    let mut files = std::collections::BTreeMap::new();
    let mut directories = vec![root.to_path_buf()];
    let mut total_bytes = 0_u64;
    while let Some(directory) = directories.pop() {
        let mut entries = fs::read_dir(directory).await?;
        while let Some(entry) = entries.next_entry().await? {
            let kind = entry.file_type().await?;
            anyhow::ensure!(
                !kind.is_symlink(),
                "memory snapshot contains a symbolic link"
            );
            if kind.is_dir() {
                directories.push(entry.path());
            } else if kind.is_file() {
                total_bytes += entry.metadata().await?.len();
                anyhow::ensure!(
                    total_bytes <= 16 * 1024 * 1024,
                    "memory snapshot exceeds size budget"
                );
                let path = entry.path();
                let mut bytes = fs::read(&path).await?;
                if path.extension().is_some_and(|extension| extension == "md") {
                    bytes = String::from_utf8(bytes)?
                        .replace(&format!("{}/", root.display()), "<memory-root>/")
                        .into_bytes();
                }
                files.insert(path.strip_prefix(root)?.to_path_buf(), bytes);
            }
        }
    }
    Ok(files)
}

#[cfg(test)]
#[path = "publication_tests.rs"]
mod tests;

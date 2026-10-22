use codex_utils_output_truncation::approx_tokens_from_byte_count_i64;
use std::path::Path;
use tokio::fs;

pub(crate) const RESOURCE_POLICY: &str = "workload-budget-v4";

#[derive(Clone, Copy, Debug)]
pub(crate) struct ConsolidationBudget {
    pub(crate) input_bytes: u64,
    pub(crate) total_tokens: i64,
}

impl ConsolidationBudget {
    pub(crate) async fn for_workspace(root: &Path) -> anyhow::Result<Self> {
        let mut input_bytes = 0_u64;
        let mut directories = vec![root.to_path_buf()];
        while let Some(directory) = directories.pop() {
            let mut entries = fs::read_dir(directory).await?;
            while let Some(entry) = entries.next_entry().await? {
                let name = entry.file_name();
                if name.to_string_lossy().starts_with('.')
                    || name == crate::workspace_diff::FILENAME
                {
                    continue;
                }
                let kind = entry.file_type().await?;
                anyhow::ensure!(!kind.is_symlink(), "memory input contains a symbolic link");
                if kind.is_dir() {
                    directories.push(entry.path());
                } else if kind.is_file() {
                    input_bytes = input_bytes.saturating_add(entry.metadata().await?.len());
                    anyhow::ensure!(
                        input_bytes <= 16 * 1024 * 1024,
                        "memory inputs exceed size budget"
                    );
                }
            }
        }
        let input_tokens = approx_tokens_from_byte_count_i64(input_bytes as i64);
        Ok(Self {
            input_bytes,
            total_tokens: 480_000_i64
                .saturating_add(input_tokens.saturating_mul(24))
                .min(2_400_000),
        })
    }
}

#[cfg(test)]
#[path = "budget_tests.rs"]
mod tests;

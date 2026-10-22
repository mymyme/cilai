use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct MemoryPreferences {
    pub enabled: bool,
    pub from_tool_chats: bool,
}

impl MemoryPreferences {
    pub(crate) fn disabled() -> Self {
        Self {
            enabled: false,
            from_tool_chats: false,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(clippy::struct_excessive_bools)]
pub struct MemorySettings {
    pub lifecycle_ready: bool,
    pub enabled: bool,
    pub from_tool_chats: bool,
    pub running: bool,
    pub has_memory: bool,
    pub model_label: Option<String>,
    pub last_error: Option<String>,
    pub phase: String,
    pub last_updated_at: Option<i64>,
    pub service: Option<MemoryServiceDisclosure>,
    pub service_error: Option<String>,
    pub native_scope_count: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryServiceDisclosure {
    pub binding_id: String,
    pub binding_revision: u32,
    pub connection_id: String,
    pub connection_name: String,
    pub config_revision: u32,
    pub auth_generation: u32,
    pub model_id: String,
    pub service_name: String,
    pub base_url: Option<String>,
    pub history_scope: String,
    pub tool_chats_history_scope: String,
    pub may_incur_cost: bool,
    pub consent_digest: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MemoryEnableConfirmation {
    pub consent_digest: String,
    pub from_tool_chats: bool,
    pub confirmed_at: i64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MemorySettingsSaveRequest {
    pub enabled: bool,
    pub from_tool_chats: bool,
    pub confirmation: Option<MemoryEnableConfirmation>,
}

pub(crate) struct NativeMemoryFiles {
    pub has_memory: bool,
    pub last_updated_at: Option<i64>,
}

pub(crate) fn native_files(harness_home: &Path) -> Result<NativeMemoryFiles, String> {
    let root = harness_home.join("memories");
    let mut modified = None;
    for name in ["MEMORY.md", "memory_summary.md"] {
        let path = root.join(name);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(NativeMemoryFiles {
                    has_memory: false,
                    last_updated_at: None,
                });
            }
            Err(_) => return Err("ai_memory_read_failed".into()),
        };
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err("ai_memory_path_denied".into());
        }
        let home = harness_home
            .canonicalize()
            .map_err(|_| "ai_memory_read_failed")?;
        if root.canonicalize().map_err(|_| "ai_memory_read_failed")? != home.join("memories") {
            return Err("ai_memory_path_denied".into());
        }
        if metadata.len() == 0 {
            return Ok(NativeMemoryFiles {
                has_memory: false,
                last_updated_at: None,
            });
        }
        let timestamp = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .and_then(|time| i64::try_from(time.as_secs()).ok());
        modified = modified.max(timestamp);
    }
    Ok(NativeMemoryFiles {
        has_memory: true,
        last_updated_at: modified,
    })
}

#[cfg(test)]
#[path = "memory_tests.rs"]
mod tests;

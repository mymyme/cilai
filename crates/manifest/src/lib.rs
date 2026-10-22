#![doc = "`.cilai/` 便携清单的稳定 Schema 与确定性序列化。"]

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const MANIFEST_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Checksum {
    pub algorithm: String,
    pub value: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceManifest {
    pub schema_version: u32,
    pub database_export_revision: i64,
    pub workspace_id: String,
    pub project_id: Option<String>,
    pub platform_root: Option<String>,
    pub local_binding_id: String,
    pub environment: String,
    pub created_at: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BaselineEntry {
    pub document_id: String,
    pub relative_path: String,
    pub baseline_version_id: String,
    pub checksum: Checksum,
    pub materialized_at: Option<i64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BaselineManifest {
    pub schema_version: u32,
    pub database_export_revision: i64,
    pub files: Vec<BaselineEntry>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ConfirmationEntry {
    pub relative_path: String,
    pub document_id: Option<String>,
    pub confirmation_id: String,
    pub confirmed_checksum: Checksum,
    pub baseline_version_id: Option<String>,
    pub confirmed_by: String,
    pub confirmed_at: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ConfirmationManifest {
    pub schema_version: u32,
    pub database_export_revision: i64,
    pub confirmations: Vec<ConfirmationEntry>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ExportState {
    pub schema_version: u32,
    pub database_export_revision: i64,
    pub generated_at: i64,
    pub manifest_checksum: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManifestSet {
    pub workspace: WorkspaceManifest,
    pub baseline: BaselineManifest,
    pub confirmation: ConfirmationManifest,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EncodedManifestSet {
    pub workspace_json: Vec<u8>,
    pub baseline_json: Vec<u8>,
    pub confirmation_json: Vec<u8>,
    pub export_state_json: Vec<u8>,
    pub manifest_checksum: String,
    pub database_export_revision: i64,
}

#[derive(Debug)]
pub enum ManifestError {
    Invalid(&'static str),
    Serialize(serde_json::Error),
}

impl std::fmt::Display for ManifestError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(reason) => write!(formatter, "invalid manifest: {reason}"),
            Self::Serialize(error) => write!(formatter, "serialize manifest: {error}"),
        }
    }
}

impl std::error::Error for ManifestError {}

impl ManifestSet {
    /// 规范化排序并生成四类清单；前三类内容摘要写入最后提交的 export-state。
    ///
    /// # Errors
    ///
    /// Schema、revision、必填标识或 checksum 不合法，或 JSON 序列化失败时返回错误。
    pub fn encode(mut self, generated_at: i64) -> Result<EncodedManifestSet, ManifestError> {
        self.validate(generated_at)?;
        self.baseline.files.sort_by(|left, right| {
            left.relative_path
                .cmp(&right.relative_path)
                .then_with(|| left.document_id.cmp(&right.document_id))
        });
        self.confirmation.confirmations.sort_by(|left, right| {
            left.relative_path
                .cmp(&right.relative_path)
                .then_with(|| left.confirmation_id.cmp(&right.confirmation_id))
        });
        let workspace_json = pretty_json(&self.workspace)?;
        let baseline_json = pretty_json(&self.baseline)?;
        let confirmation_json = pretty_json(&self.confirmation)?;
        let manifest_checksum = checksum_group([
            workspace_json.as_slice(),
            baseline_json.as_slice(),
            confirmation_json.as_slice(),
        ]);
        let export_state_json = pretty_json(&ExportState {
            schema_version: MANIFEST_SCHEMA_VERSION,
            database_export_revision: self.workspace.database_export_revision,
            generated_at,
            manifest_checksum: manifest_checksum.clone(),
        })?;
        Ok(EncodedManifestSet {
            workspace_json,
            baseline_json,
            confirmation_json,
            export_state_json,
            manifest_checksum,
            database_export_revision: self.workspace.database_export_revision,
        })
    }

    fn validate(&self, generated_at: i64) -> Result<(), ManifestError> {
        let revision = self.workspace.database_export_revision;
        if self.workspace.schema_version != MANIFEST_SCHEMA_VERSION
            || self.baseline.schema_version != MANIFEST_SCHEMA_VERSION
            || self.confirmation.schema_version != MANIFEST_SCHEMA_VERSION
        {
            return Err(ManifestError::Invalid("unsupported schema version"));
        }
        if revision < 0
            || self.baseline.database_export_revision != revision
            || self.confirmation.database_export_revision != revision
        {
            return Err(ManifestError::Invalid("inconsistent export revision"));
        }
        if generated_at < 0
            || self.workspace.workspace_id.is_empty()
            || self.workspace.local_binding_id.is_empty()
        {
            return Err(ManifestError::Invalid("missing workspace identity or time"));
        }
        let invalid_baseline = self.baseline.files.iter().any(|entry| {
            entry.document_id.is_empty()
                || entry.relative_path.is_empty()
                || entry.baseline_version_id.is_empty()
                || !valid_checksum(&entry.checksum)
        });
        let invalid_confirmation = self.confirmation.confirmations.iter().any(|entry| {
            entry.relative_path.is_empty()
                || entry.confirmation_id.is_empty()
                || entry.confirmed_by.is_empty()
                || entry.confirmed_at < 0
                || !valid_checksum(&entry.confirmed_checksum)
        });
        if invalid_baseline || invalid_confirmation {
            return Err(ManifestError::Invalid("invalid manifest entry"));
        }
        Ok(())
    }
}

fn valid_checksum(checksum: &Checksum) -> bool {
    !checksum.algorithm.is_empty() && !checksum.value.is_empty()
}

fn pretty_json<T: Serialize>(value: &T) -> Result<Vec<u8>, ManifestError> {
    let mut bytes = serde_json::to_vec_pretty(value).map_err(ManifestError::Serialize)?;
    bytes.push(b'\n');
    Ok(bytes)
}

#[must_use]
pub fn checksum_group<const N: usize>(documents: [&[u8]; N]) -> String {
    let mut digest = Sha256::new();
    for document in documents {
        digest.update((document.len() as u64).to_be_bytes());
        digest.update(document);
    }
    format!("{:x}", digest.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoding_is_sorted_deterministic_and_export_state_commits_group_checksum() {
        let set = ManifestSet {
            workspace: WorkspaceManifest {
                schema_version: MANIFEST_SCHEMA_VERSION,
                database_export_revision: 7,
                workspace_id: "workspace-1".to_owned(),
                project_id: None,
                platform_root: None,
                local_binding_id: "workspace-1".to_owned(),
                environment: "local".to_owned(),
                created_at: 1,
            },
            baseline: BaselineManifest {
                schema_version: MANIFEST_SCHEMA_VERSION,
                database_export_revision: 7,
                files: vec![baseline("z.md"), baseline("a.md")],
            },
            confirmation: ConfirmationManifest {
                schema_version: MANIFEST_SCHEMA_VERSION,
                database_export_revision: 7,
                confirmations: Vec::new(),
            },
        };
        let encoded = set.encode(9).unwrap();
        let baseline: BaselineManifest = serde_json::from_slice(&encoded.baseline_json).unwrap();
        let state: ExportState = serde_json::from_slice(&encoded.export_state_json).unwrap();
        assert_eq!(baseline.files[0].relative_path, "a.md");
        assert_eq!(state.manifest_checksum, encoded.manifest_checksum);
        assert_eq!(state.database_export_revision, 7);
    }

    #[test]
    fn inconsistent_revisions_and_incomplete_entries_are_rejected() {
        let mut set = valid_set();
        set.baseline.database_export_revision = 2;
        assert!(matches!(set.encode(1), Err(ManifestError::Invalid(_))));
        let mut set = valid_set();
        set.baseline.files[0].checksum.value.clear();
        assert!(matches!(set.encode(1), Err(ManifestError::Invalid(_))));
    }

    fn valid_set() -> ManifestSet {
        ManifestSet {
            workspace: WorkspaceManifest {
                schema_version: MANIFEST_SCHEMA_VERSION,
                database_export_revision: 1,
                workspace_id: "w".to_owned(),
                project_id: None,
                platform_root: None,
                local_binding_id: "w".to_owned(),
                environment: "local".to_owned(),
                created_at: 0,
            },
            baseline: BaselineManifest {
                schema_version: MANIFEST_SCHEMA_VERSION,
                database_export_revision: 1,
                files: vec![baseline("a.md")],
            },
            confirmation: ConfirmationManifest {
                schema_version: MANIFEST_SCHEMA_VERSION,
                database_export_revision: 1,
                confirmations: Vec::new(),
            },
        }
    }

    fn baseline(path: &str) -> BaselineEntry {
        BaselineEntry {
            document_id: format!("document-{path}"),
            relative_path: path.to_owned(),
            baseline_version_id: "version-1".to_owned(),
            checksum: Checksum {
                algorithm: "sha256".to_owned(),
                value: "abc".to_owned(),
            },
            materialized_at: Some(1),
        }
    }
}

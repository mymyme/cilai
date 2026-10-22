use super::*;
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactVersion {
    pub digest: String,
    pub size_bytes: u64,
    pub created_at: i64,
    pub snapshot_path: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArtifactFeedback {
    pub feedback_id: String,
    pub version_digest: String,
    pub content: String,
    pub created_at: i64,
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub(super) fn freeze(data_dir: &Path, artifact: &mut AgentRunArtifact) -> Result<(), String> {
    if artifact.version.is_some() {
        return Ok(());
    }
    let source = Path::new(artifact.path.as_deref().ok_or("ai_artifact_path_missing")?);
    let metadata = fs::metadata(source).map_err(|_| "ai_artifact_unavailable")?;
    if !metadata.is_file() || metadata.len() > 128 * 1024 * 1024 {
        return Err("ai_artifact_version_too_large".into());
    }
    let bytes = fs::read(source).map_err(|_| "ai_artifact_unavailable")?;
    if bytes.len() > 128 * 1024 * 1024 {
        return Err("ai_artifact_version_too_large".into());
    }
    let version_digest = digest(&bytes);
    let directory = data_dir
        .join("artifact-versions")
        .join(&artifact.artifact_id);
    fs::create_dir_all(&directory).map_err(|_| "ai_artifact_snapshot_failed")?;
    let path = directory.join(source.file_name().ok_or("ai_artifact_path_missing")?);
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|_| "ai_artifact_snapshot_failed")?;
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(|_| "ai_artifact_snapshot_failed")?;
    let mut permissions = file
        .metadata()
        .map_err(|_| "ai_artifact_snapshot_failed")?
        .permissions();
    permissions.set_readonly(true);
    fs::set_permissions(&path, permissions).map_err(|_| "ai_artifact_snapshot_failed")?;
    artifact.version = Some(ArtifactVersion {
        digest: version_digest,
        size_bytes: bytes.len() as u64,
        created_at: unix_epoch_millis()?,
        snapshot_path: path.to_string_lossy().into_owned(),
    });
    Ok(())
}

pub(super) fn verified_path(data_dir: &Path, version: &ArtifactVersion) -> Result<PathBuf, String> {
    let root = data_dir
        .join("artifact-versions")
        .canonicalize()
        .map_err(|_| "ai_artifact_snapshot_missing")?;
    let path = Path::new(&version.snapshot_path)
        .canonicalize()
        .map_err(|_| "ai_artifact_snapshot_missing")?;
    if !path.starts_with(root) {
        return Err("ai_artifact_scope_invalid".into());
    }
    let metadata = fs::metadata(&path).map_err(|_| "ai_artifact_snapshot_missing")?;
    if !metadata.is_file()
        || metadata.len() != version.size_bytes
        || metadata.len() > 128 * 1024 * 1024
    {
        return Err("ai_artifact_version_changed".into());
    }
    let bytes = fs::read(&path).map_err(|_| "ai_artifact_snapshot_missing")?;
    if digest(&bytes) != version.digest {
        return Err("ai_artifact_version_changed".into());
    }
    Ok(path)
}

impl AiRuntimeManager {
    /// # Errors
    /// Returns an error when the version, request, or persistence is unavailable.
    pub fn artifact_feedback(
        &self,
        run_id: &str,
        artifact_id: &str,
        version_digest: &str,
        feedback_id: &str,
        content: &str,
    ) -> Result<Value, String> {
        if content.trim().is_empty()
            || content.chars().count() > 10000
            || !safe_storage_id(feedback_id)
        {
            return Err("ai_artifact_feedback_invalid".into());
        }
        let mut state = lock(&self.inner);
        let data_dir = data_dir_for_state(&state)?;
        let previous = state.persisted.clone();
        let run = state
            .persisted
            .runs
            .iter_mut()
            .find(|run| run.run_id == run_id)
            .ok_or("agent_run_not_found")?;
        let artifact = run
            .artifacts
            .iter_mut()
            .find(|artifact| artifact.artifact_id == artifact_id)
            .ok_or("ai_artifact_not_found")?;
        let version = artifact
            .version
            .as_ref()
            .filter(|version| version.digest == version_digest)
            .ok_or("ai_artifact_version_mismatch")?;
        verified_path(&data_dir, version)?;
        if let Some(existing) = artifact
            .feedback
            .iter()
            .find(|item| item.feedback_id == feedback_id)
        {
            if existing.version_digest != version_digest || existing.content != content.trim() {
                return Err("ai_artifact_feedback_conflict".into());
            }
            return serde_json::to_value(existing).map_err(|error| error.to_string());
        }
        let feedback = ArtifactFeedback {
            feedback_id: feedback_id.into(),
            version_digest: version_digest.into(),
            content: content.trim().into(),
            created_at: unix_epoch_millis()?,
        };
        artifact.feedback.push(feedback.clone());
        persist_locked_or_restore(&mut state, previous)?;
        serde_json::to_value(feedback).map_err(|error| error.to_string())
    }

    /// # Errors
    /// Returns an error when the version, request, or persistence is unavailable.
    pub fn submit_artifact_candidate(
        &self,
        run_id: &str,
        artifact_id: &str,
        version_digest: &str,
        deliverable_id: &str,
        summary: &str,
        operation_id: &str,
    ) -> Result<Value, String> {
        if summary.trim().is_empty()
            || summary.chars().count() > 10000
            || !safe_storage_id(operation_id)
        {
            return Err("ai_artifact_candidate_invalid".into());
        }
        let state = lock(&self.inner);
        let run = state
            .persisted
            .runs
            .iter()
            .find(|run| run.run_id == run_id && run.status == "completed" && !run.superseded)
            .ok_or("ai_artifact_run_unavailable")?;
        let conversation = state
            .persisted
            .conversations
            .iter()
            .find(|item| item.conversation_id == run.conversation_id)
            .ok_or("conversation_not_found")?;
        let project_id = conversation
            .context
            .project_id
            .as_deref()
            .ok_or("ai_artifact_project_required")?;
        let artifact = run
            .artifacts
            .iter()
            .find(|item| item.artifact_id == artifact_id)
            .ok_or("ai_artifact_not_found")?;
        let version = artifact
            .version
            .as_ref()
            .filter(|version| version.digest == version_digest)
            .ok_or("ai_artifact_version_mismatch")?;
        verified_path(&data_dir_for_state(&state)?, version)?;
        let server = state.server.as_ref().ok_or("ai_runtime_not_initialized")?;
        let catalog = server.request_result(
            "deliverable/list",
            &serde_json::json!({"projectId":project_id}),
        )?;
        if !catalog["deliverables"].as_array().is_some_and(|items| {
            items
                .iter()
                .any(|item| item["deliverableId"] == deliverable_id)
        }) {
            return Err("ai_artifact_deliverable_scope_invalid".into());
        }
        let reference = format!("cilai-artifact:{run_id}/{artifact_id}/{version_digest}");
        server.request_result("deliverable/version/create", &serde_json::json!({
            "deliverableId": deliverable_id, "summary": summary.trim(), "localFileIds": [],
            "carriers": [{"carrierType":"external", "carrierRole":"primary", "label":format!("本机产物 · {}",artifact.label), "reference":reference, "versionLabel":version_digest, "sourceVersionId":null}],
            "clientOperationId":operation_id, "requestFingerprint":sha256_hex(&format!("{deliverable_id}\n{reference}\n{}",summary.trim()))
        }))
    }
}

pub(super) fn ensure_conversations_unreferenced(
    persisted: &PersistedAiState,
    server: &AppServerSupervisor,
    conversation_ids: &[String],
) -> Result<(), String> {
    for conversation in persisted
        .conversations
        .iter()
        .filter(|item| conversation_ids.contains(&item.conversation_id))
    {
        if let Some(project_id) = &conversation.context.project_id {
            let catalog = server.request_result(
                "deliverable/list",
                &serde_json::json!({"projectId":project_id}),
            )?;
            let linked = catalog["deliverables"]
                .as_array()
                .into_iter()
                .flatten()
                .flat_map(|item| item["versions"].as_array().into_iter().flatten())
                .flat_map(|item| item["carriers"].as_array().into_iter().flatten())
                .any(|carrier| {
                    carrier["reference"].as_str().is_some_and(|reference| {
                        persisted.runs.iter().any(|run| {
                            run.conversation_id == conversation.conversation_id
                                && reference.starts_with(&format!("cilai-artifact:{}/", run.run_id))
                        })
                    })
                });
            if linked {
                return Err("此对话包含交付物引用的产物版本，请保留归档记录。".into());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn snapshot_preserves_original_and_rejects_tampering() {
        let root = std::env::temp_dir().join(format!("artifact-version-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let source = root.join("result.md");
        fs::write(&source, "original").unwrap();
        let mut artifact = AgentRunArtifact {
            artifact_id: "a1".into(),
            artifact_type: "document".into(),
            label: "result.md".into(),
            path: Some(source.to_string_lossy().into_owned()),
            version: None,
            feedback: Vec::new(),
        };
        freeze(&root, &mut artifact).unwrap();
        fs::write(&source, "new content").unwrap();
        let version = artifact.version.unwrap();
        let snapshot = verified_path(&root, &version).unwrap();
        assert_eq!(fs::read_to_string(&snapshot).unwrap(), "original");
        fs::remove_file(&snapshot).unwrap();
        fs::write(&snapshot, "tampered").unwrap();
        assert_eq!(
            verified_path(&root, &version).unwrap_err(),
            "ai_artifact_version_changed"
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    #[expect(clippy::too_many_lines)]
    fn feedback_and_candidate_bind_exact_version_and_survive_restart() {
        let _guard = crate::supervisor::app_server_test_guard();
        let owner = super::super::model_test_fixture::fixture_root("artifact-review");
        let root = super::super::model_test_fixture::profile_data_root(&owner);
        let server = AppServerSupervisor::new();
        server
            .start(
                &crate::supervisor::app_server_test_binary(),
                "test",
                &root,
                &root.join("run"),
            )
            .unwrap();
        let manager = AiRuntimeManager::new();
        manager.initialize(&root, &server).unwrap();
        let workspace_root = owner.join("report-workspace");
        fs::create_dir_all(&workspace_root).unwrap();
        let workspace = server.request_result("workspace/create", &serde_json::json!({"localRoot":workspace_root,"displayName":"报告工作空间","clientOperationId":"report-workspace"})).unwrap();
        let project = server.request_result("project/create", &serde_json::json!({"workspaceId":workspace["workspace"]["workspaceId"],"workspaceIds":[],"displayName":"报告项目","goal":"复核报告","clientOperationId":"create-project","requestFingerprint":"create-project"})).unwrap();
        let project_id = project["project"]["projectId"].as_str().unwrap();
        let conversation = manager.create_conversation(serde_json::from_value(serde_json::json!({"title":"整理报告","conversationType":"project","mode":"discuss","agentId":null,"context":{"workspaceRoot":workspace_root,"workspaceId":workspace["workspace"]["workspaceId"],"projectId":project_id}})).unwrap()).unwrap();
        let deliverable = server.request_result("deliverable/create", &serde_json::json!({"projectId":project_id,"title":"报告","deliverableType":"document","ownerLabel":"测试员工","acceptanceCriteria":"人工核对数据","clientOperationId":"create-deliverable","requestFingerprint":"create-deliverable"})).unwrap();
        let deliverable_id = deliverable["deliverable"]["deliverableId"]
            .as_str()
            .unwrap();
        let source = workspace_root.join("report.md");
        fs::write(&source, "第一版报告").unwrap();
        let mut artifact = AgentRunArtifact {
            artifact_id: "artifact-test".into(),
            artifact_type: "document".into(),
            label: "报告".into(),
            path: Some(source.to_string_lossy().into_owned()),
            version: None,
            feedback: Vec::new(),
        };
        freeze(
            &data_dir_for_state(&lock(&manager.inner)).unwrap(),
            &mut artifact,
        )
        .unwrap();
        let digest = artifact.version.as_ref().unwrap().digest.clone();
        let run: AgentRun = serde_json::from_value(serde_json::json!({"runId":"run-test","conversationId":conversation.conversation_id,"agentId":"model","inputMessageId":"message-test","provider":"model","status":"completed","startedAt":1,"completedAt":2,"artifacts":[artifact]})).unwrap();
        lock(&manager.inner).persisted.runs.push(run);
        let feedback = manager
            .artifact_feedback(
                "run-test",
                "artifact-test",
                &digest,
                "feedback-test",
                "请补充依据",
            )
            .unwrap();
        assert_eq!(
            manager
                .artifact_feedback(
                    "run-test",
                    "artifact-test",
                    &digest,
                    "feedback-test",
                    "请补充依据"
                )
                .unwrap(),
            feedback
        );
        assert_eq!(
            manager
                .artifact_feedback(
                    "run-test",
                    "artifact-test",
                    &digest,
                    "feedback-test",
                    "其他内容"
                )
                .unwrap_err(),
            "ai_artifact_feedback_conflict"
        );
        assert_eq!(
            manager
                .artifact_feedback(
                    "run-test",
                    "artifact-test",
                    "other",
                    "feedback-new",
                    "补充依据"
                )
                .unwrap_err(),
            "ai_artifact_version_mismatch"
        );
        assert_eq!(
            manager
                .submit_artifact_candidate(
                    "run-test",
                    "artifact-test",
                    &digest,
                    "other-project",
                    "候选报告",
                    "operation-test"
                )
                .unwrap_err(),
            "ai_artifact_deliverable_scope_invalid"
        );
        fs::write(&source, "第二版报告").unwrap();
        let submitted = manager
            .submit_artifact_candidate(
                "run-test",
                "artifact-test",
                &digest,
                deliverable_id,
                "候选报告",
                "operation-test",
            )
            .unwrap();
        assert_eq!(
            manager
                .submit_artifact_candidate(
                    "run-test",
                    "artifact-test",
                    &digest,
                    deliverable_id,
                    "候选报告",
                    "operation-test"
                )
                .unwrap(),
            submitted
        );
        let listed = server
            .request_result(
                "deliverable/list",
                &serde_json::json!({"projectId":project_id}),
            )
            .unwrap();
        assert_eq!(
            listed["deliverables"][0]["versions"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_ne!(listed["deliverables"][0]["status"], "accepted");
        manager.shutdown_checked().unwrap();
        let reloaded = AiRuntimeManager::new();
        reloaded.initialize(&root, &server).unwrap();
        assert_eq!(
            lock(&reloaded.inner).persisted.runs[0].artifacts[0]
                .feedback
                .len(),
            1
        );
        assert_eq!(
            reloaded
                .artifact_feedback(
                    "run-test",
                    "artifact-test",
                    &digest,
                    "feedback-test",
                    "请补充依据"
                )
                .unwrap(),
            feedback
        );
        reloaded.shutdown_checked().unwrap();
        server.shutdown().unwrap();
        fs::remove_dir_all(owner).unwrap();
    }
}

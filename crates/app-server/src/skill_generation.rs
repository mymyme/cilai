use super::{AppServerServices, ServiceError, SharedAppServerServices, skills};
use contexthub_persistence_sqlite::SkillInstallationRecord;
use contexthub_protocol::{
    SkillImportDirectoryParams, SkillPackageFileInput, SkillRunInstallation, SkillRunListParams,
    SkillRunListResult, SkillRunRetryParams, SkillSaveParams,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::time::Duration;

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Draft {
    name: String,
    description: String,
    instructions: String,
    files: Vec<SkillPackageFileInput>,
}

struct Delivery {
    draft: Draft,
    display_content: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct DraftError {
    reason: String,
    detail: String,
}

fn draft_error(reason: &'static str, detail: impl Into<String>) -> DraftError {
    DraftError {
        reason: reason.to_owned(),
        detail: detail.into(),
    }
}

fn fence(line: &str) -> Option<(u8, usize, &str)> {
    let line = line.trim_end_matches(['\r', '\n']);
    let indent = line.bytes().take_while(|byte| *byte == b' ').count();
    if indent > 3 {
        return None;
    }
    let line = &line[indent..];
    let marker = *line.as_bytes().first()?;
    if !matches!(marker, b'`' | b'~') {
        return None;
    }
    let count = line.bytes().take_while(|byte| *byte == marker).count();
    (count >= 3).then(|| (marker, count, line[count..].trim()))
}

fn parse_delivery(content: &str) -> Result<Option<Delivery>, DraftError> {
    let mut active: Option<(u8, usize, bool, usize, usize)> = None;
    let mut block = None;
    let mut offset = 0;
    for line in content.split_inclusive('\n') {
        if let Some((marker, count, info)) = fence(line) {
            if let Some((opening, length, target, start, body_start)) = active {
                if marker == opening && count >= length && info.is_empty() {
                    if target {
                        if block.is_some() {
                            return Err(draft_error(
                                "skill_multiple_deliveries",
                                "一次回复包含多个技能包，请分别交付。",
                            ));
                        }
                        block = Some((start, body_start, offset, offset + line.len()));
                    }
                    active = None;
                }
            } else {
                active = Some((
                    marker,
                    count,
                    info.eq_ignore_ascii_case("cilai-skill"),
                    offset,
                    offset + line.len(),
                ));
            }
        }
        offset += line.len();
    }
    if active.is_some_and(|(_, _, target, _, _)| target) {
        return Err(draft_error(
            "skill_delivery_incomplete",
            "技能内容未完整输出，请继续生成完整技能包。",
        ));
    }
    let Some((start, body_start, body_end, end)) = block else {
        return Ok(None);
    };
    let body = &content[body_start..body_end];
    if body.len() > 2 * 1024 * 1024 {
        return Err(draft_error(
            "skill_delivery_too_large",
            "技能交付内容超过 2 MiB。",
        ));
    }
    let draft: Draft = serde_json::from_str(body).map_err(|error| {
        let reason = if error.is_eof() {
            "skill_delivery_incomplete"
        } else if error.is_data() {
            "skill_delivery_fields_invalid"
        } else {
            "skill_delivery_json_invalid"
        };
        draft_error(
            reason,
            format!(
                "技能数据第 {} 行、第 {} 列：{error}",
                error.line(),
                error.column()
            ),
        )
    })?;
    Ok(Some(Delivery {
        draft,
        display_content: format!("{}{}", &content[..start], &content[end..])
            .trim()
            .to_owned(),
    }))
}

fn message_content(source: &Value) -> String {
    let content = source["message"]["content"].as_str().unwrap_or_default();
    if let Ok(value) = serde_json::from_str::<Value>(content)
        && value.as_object().is_some_and(|object| object.len() == 4)
        && value["schemaVersion"] == 1
        && value["nonce"].is_string()
        && value["classification"].is_object()
        && let Some(reply) = value["reply"].as_str()
    {
        return reply.to_owned();
    }
    content.to_owned()
}

pub(super) fn structured_delivery(content: &str) -> Value {
    json!(parse_delivery(content).map(|delivery| delivery.map(|delivery| delivery.draft)))
}

fn source_delivery(source: &Value) -> Result<Option<Delivery>, DraftError> {
    if let Some(value) = source.get("skillDelivery") {
        let delivery: Result<Option<Draft>, DraftError> = serde_json::from_value(value.clone())
            .map_err(|_| draft_error("skill_delivery_state_invalid", "技能交付记录无法读取。"))?;
        return delivery.map(|draft| {
            draft.map(|draft| Delivery {
                draft,
                display_content: String::new(),
            })
        });
    }
    parse_delivery(&message_content(source))
}

fn result_for(record: &SkillInstallationRecord, source: &Value) -> SkillRunInstallation {
    SkillRunInstallation {
        run_id: record.run_id.clone(),
        message_id: source["message"]["messageId"].as_str().map(str::to_owned),
        status: record.status.clone(),
        skill_id: None,
        name: None,
        error_reason: None,
        error_detail: None,
        retryable: false,
        display_content: None,
    }
}

fn directory_source(source: &Value) -> Result<Option<(&str, &str)>, DraftError> {
    let Some(root) = source["workspaceRoot"].as_str() else {
        return Ok(None);
    };
    let Some(artifacts) = source["artifacts"].as_array() else {
        return Ok(None);
    };
    let mut candidates = artifacts
        .iter()
        .filter_map(|artifact| artifact["path"].as_str())
        .filter(|path| {
            let path = Path::new(path);
            path.starts_with(root)
                && path.file_name().is_some_and(|name| name == "SKILL.md")
                && path
                    .parent()
                    .and_then(Path::parent)
                    .and_then(Path::file_name)
                    .is_some_and(|name| name == "skills")
        });
    let first = candidates.next();
    if candidates.next().is_some() {
        return Err(draft_error(
            "skill_multiple_deliveries",
            "一次运行登记了多个技能目录，请分别添加。",
        ));
    }
    Ok(first.map(|path| (path, root)))
}

enum Prepared {
    Json(SkillSaveParams),
    Directory(SkillImportDirectoryParams),
    None,
}

fn prepare(
    record: &SkillInstallationRecord,
    source: &Value,
    result: &mut SkillRunInstallation,
) -> Result<Prepared, DraftError> {
    let operation = format!("desktop-ai-skill-{}", record.run_id);
    if let Some(delivery) = source_delivery(source)? {
        let draft = delivery.draft;
        if source.get("skillDelivery").is_none() {
            result.display_content = Some(delivery.display_content);
        }
        result.name = Some(draft.name.trim().to_owned());
        let fingerprint = format!("{:x}", Sha256::digest(json!({"name": draft.name, "description": draft.description, "instructions": draft.instructions, "files": draft.files}).to_string()));
        let params = SkillSaveParams {
            skill_id: None,
            name: draft.name,
            description: draft.description,
            instructions: draft.instructions,
            files: Some(draft.files),
            enabled: true,
            automatic: false,
            expected_revision: None,
            client_operation_id: operation,
            request_fingerprint: fingerprint,
        };
        skills::validate_save(&params)
            .map_err(|error| draft_error(error.reason, validation_detail(error.reason)))?;
        return Ok(Prepared::Json(params));
    }
    if let Some((path, root)) = directory_source(source)? {
        return Ok(Prepared::Directory(SkillImportDirectoryParams {
            skill_file_path: path.to_owned(),
            workspace_root: root.to_owned(),
            client_operation_id: operation,
            request_fingerprint: format!(
                "{:x}",
                Sha256::digest(json!({"skillFilePath": path, "workspaceRoot": root}).to_string())
            ),
        }));
    }
    Ok(Prepared::None)
}

fn validation_detail(reason: &str) -> &'static str {
    match reason {
        "skill_name_invalid" => "技能名称不能为空，且不能超过 80 个字符。",
        "skill_description_invalid" => "技能简介不能为空，且不能超过 240 个字符。",
        "skill_instructions_invalid" => "技能正文不能为空，且不能超过 50000 个字符。",
        "skill_file_path_invalid" => {
            "资源必须位于 scripts、references 或 assets 中，路径不能越界。"
        }
        "skill_file_content_invalid" => "单个资源超过 256 KiB 或包含无效字符。",
        "skill_files_invalid" => "资源超过 64 个、总大小超过 1 MiB，或存在重复路径。",
        _ => "技能内容包含无效字符，请修正后重新交付。",
    }
}

impl AppServerServices {
    pub(super) fn skill_run_list(
        &mut self,
        params: &SkillRunListParams,
    ) -> Result<SkillRunListResult, ServiceError> {
        if params.run_ids.len() > 256 {
            return Err(ServiceError::new("skill_run_ids_invalid"));
        }
        let mut installations = Vec::new();
        for id in &params.run_ids {
            if let Some(record) = self
                .persistence()?
                .skill_installation(id)
                .map_err(skills::skill_persistence_error)?
            {
                let result = self.skill_installation_view(&record)?;
                if result.status != "ignored" {
                    installations.push(result);
                }
            }
        }
        Ok(SkillRunListResult { installations })
    }

    fn skill_installation_view(
        &self,
        record: &SkillInstallationRecord,
    ) -> Result<SkillRunInstallation, ServiceError> {
        let source: Value = serde_json::from_str(&record.source_json)
            .map_err(|_| ServiceError::new("skill_installation_state_invalid"))?;
        let mut result = if let Some(saved) = &record.result_json {
            serde_json::from_str(saved)
                .map_err(|_| ServiceError::new("skill_installation_state_invalid"))?
        } else {
            result_for(record, &source)
        };
        if matches!(record.status.as_str(), "historical" | "pending") {
            match prepare(record, &source, &mut result) {
                Ok(Prepared::None) => "ignored".clone_into(&mut result.status),
                Ok(_) => result.retryable = record.status == "historical",
                Err(error) => set_invalid(&mut result, error),
            }
            if let Some(operation) = self
                .persistence()?
                .user_skill_operation(&format!("desktop-ai-skill-{}", record.run_id))
                .map_err(skills::skill_persistence_error)?
            {
                result.skill_id = Some(operation.skill_id);
                "installed".clone_into(&mut result.status);
                result.retryable = false;
                result.error_reason = None;
                result.error_detail = None;
            }
        }
        if let Some(id) = &result.skill_id {
            if let Some(skill) = self
                .persistence()?
                .user_skill(id)
                .map_err(skills::skill_persistence_error)?
            {
                result.name = Some(skill.name);
            } else {
                "deleted".clone_into(&mut result.status);
                result.retryable = false;
            }
        }
        Ok(result)
    }

    pub(super) fn skill_run_retry(
        &mut self,
        params: &SkillRunRetryParams,
        now: i64,
    ) -> Result<SkillRunInstallation, ServiceError> {
        let record = self
            .persistence()?
            .skill_installation(&params.run_id)
            .map_err(skills::skill_persistence_error)?
            .ok_or_else(|| ServiceError::new("skill_run_not_found"))?;
        let existing = self.skill_installation_view(&record)?;
        if matches!(existing.status.as_str(), "installed" | "deleted") {
            return Ok(existing);
        }
        self.process_skill_installation(&record, now)
    }

    fn process_skill_installation(
        &mut self,
        record: &SkillInstallationRecord,
        now: i64,
    ) -> Result<SkillRunInstallation, ServiceError> {
        let source: Value = serde_json::from_str(&record.source_json)
            .map_err(|_| ServiceError::new("skill_installation_state_invalid"))?;
        let mut result = result_for(record, &source);
        if self
            .persistence()?
            .skill_source_run_available(&record.run_id)
            .map_err(skills::skill_persistence_error)?
        {
            self.install_prepared_skill(record, &source, &mut result, now)?;
        } else {
            "ignored".clone_into(&mut result.status);
        }
        self.persistence()?
            .finish_skill_installation(
                &record.run_id,
                &result.status,
                &serde_json::to_value(&result)
                    .map_err(|_| ServiceError::new("skill_installation_state_invalid"))?,
            )
            .map_err(skills::skill_persistence_error)?;
        Ok(result)
    }

    fn install_prepared_skill(
        &mut self,
        record: &SkillInstallationRecord,
        source: &Value,
        result: &mut SkillRunInstallation,
        now: i64,
    ) -> Result<(), ServiceError> {
        let prepared = match prepare(record, source, result) {
            Ok(prepared) => prepared,
            Err(error) => {
                set_invalid(result, error);
                return Ok(());
            }
        };
        if let Some(operation) = self
            .persistence()?
            .user_skill_operation(&format!("desktop-ai-skill-{}", record.run_id))
            .map_err(skills::skill_persistence_error)?
        {
            result.skill_id = Some(operation.skill_id);
            "installed".clone_into(&mut result.status);
            return Ok(());
        }
        let saved = match prepared {
            Prepared::Json(params) => self.skill_save(&params, now).map(|saved| saved.skill),
            Prepared::Directory(params) => self
                .skill_import_directory(&params, now)
                .map(|saved| saved.skill),
            Prepared::None => {
                "ignored".clone_into(&mut result.status);
                return Ok(());
            }
        };
        match saved {
            Ok(skill) => {
                "installed".clone_into(&mut result.status);
                result.skill_id = Some(skill.skill_id);
                result.name = Some(skill.name);
            }
            Err(error) => {
                "failed".clone_into(&mut result.status);
                result.error_reason = Some(error.reason.to_owned());
                result.retryable = !matches!(
                    error.reason,
                    "skill_name_conflict"
                        | "skill_path_invalid"
                        | "skill_file_path_invalid"
                        | "skill_file_invalid"
                        | "skill_content_invalid"
                        | "skill_files_invalid"
                        | "skill_file_content_invalid"
                        | "skill_directory_structure_invalid"
                );
            }
        }
        Ok(())
    }

    pub(super) fn process_pending_skills(&mut self, now: i64) -> Result<(), ServiceError> {
        for id in self
            .persistence()?
            .pending_skill_installations()
            .map_err(skills::skill_persistence_error)?
        {
            if let Some(record) = self
                .persistence()?
                .skill_installation(&id)
                .map_err(skills::skill_persistence_error)?
            {
                self.process_skill_installation(&record, now)?;
            }
        }
        Ok(())
    }
}

fn set_invalid(result: &mut SkillRunInstallation, error: DraftError) {
    "invalid".clone_into(&mut result.status);
    result.error_reason = Some(error.reason.clone());
    result.error_detail = Some(error.detail);
    result.retryable = false;
}

pub fn spawn(
    services: SharedAppServerServices,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(500));
        loop {
            if *shutdown.borrow() {
                break;
            }
            tokio::select! {
                _ = shutdown.changed() => break,
                _ = interval.tick() => {
                    let services = services.clone();
                    let result = tokio::task::spawn_blocking(move || {
                        let mut services = services.lock().map_err(|_| "skill_service_lock_failed")?;
                        services.process_pending_skills(super::unix_epoch_millis().map_err(|_| "skill_clock_failed")?)
                            .map_err(|error| error.reason)
                    }).await;
                    if !matches!(result, Ok(Ok(()))) { eprintln!("skill_installation_worker_failed: {result:?}"); break; }
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use contexthub_persistence_sqlite::SqlitePersistence;
    use contexthub_protocol::{AiStateReplaceParams, SkillListParams};
    use std::fs;
    use uuid::Uuid;

    fn content(name: &str) -> String {
        let draft = json!({"name":name,"description":"测试技能","instructions":"# 方法\n\n```bash\necho 'quoted'\n```\n中文说明",
            "files":[{"path":"references/example.md","content":"```python\nprint(\"中文\")\n```"}]});
        format!("交付如下。\n\n```cilai-skill\n{draft}\n```\n\n验证结束。")
    }

    fn root() -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("cilai-skill-generation-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn services(root: &Path) -> AppServerServices {
        AppServerServices::with_data_dir(
            Some(SqlitePersistence::open(&root.join("contexthub.db"), "test").unwrap()),
            Some(root.to_owned()),
        )
    }

    fn state(content: &str) -> Value {
        json!({"conversations":[{"conversationId":"conversation","context":{"workspaceRoot":"/tmp"}}],
            "runs":[{"runId":"run-test","conversationId":"conversation","status":"completed","superseded":false,"artifacts":[]}],
            "messages":[{"runId":"run-test","messageId":"first","role":"assistant","content":"处理中"},
                {"runId":"run-test","messageId":"final","role":"assistant","content":content}]})
    }

    #[test]
    fn fenced_examples_are_preserved_and_display_uses_same_boundaries() {
        let text = content("复杂技能");
        let parsed = parse_delivery(&text).unwrap().unwrap();
        assert!(parsed.draft.instructions.contains("```bash\n"));
        assert!(parsed.draft.files[0].content.contains("print(\"中文\")"));
        assert_eq!(parsed.display_content, "交付如下。\n\n\n验证结束。");
        let crlf = text.replace('\n', "\r\n");
        assert_eq!(
            parse_delivery(&crlf).unwrap().unwrap().draft.name,
            "复杂技能"
        );
        let four = text
            .replace("```cilai-skill", "````cilai-skill")
            .replace("\n```\n", "\n````\n");
        assert!(parse_delivery(&four).unwrap().is_some());
    }

    #[test]
    fn incomplete_invalid_and_multiple_deliveries_are_explicit_errors() {
        assert_eq!(
            parse_delivery("```cilai-skill\n{").err().unwrap().reason,
            "skill_delivery_incomplete"
        );
        assert_eq!(
            parse_delivery("```cilai-skill\n{\"name\":\"missing\"}\n```")
                .err()
                .unwrap()
                .reason,
            "skill_delivery_fields_invalid"
        );
        assert_eq!(
            parse_delivery("```cilai-skill\ninvalid\n```")
                .err()
                .unwrap()
                .reason,
            "skill_delivery_json_invalid"
        );
        assert_eq!(
            parse_delivery(&format!("{}\n{}", content("a"), content("b")))
                .err()
                .unwrap()
                .reason,
            "skill_multiple_deliveries"
        );
        assert!(
            parse_delivery(&format!("````markdown\n{}\n````", content("示例")))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn completed_run_installs_without_ui_and_survives_restart_without_duplicates() {
        let root = root();
        let mut server = services(&root);
        let state = state(&content("无需页面"));
        server
            .ai_state_replace(
                &AiStateReplaceParams {
                    expected_revision: 0,
                    state: state.clone(),
                },
                1,
            )
            .unwrap();
        drop(server);
        let mut server = services(&root);
        server.process_pending_skills(2).unwrap();
        let result = server
            .skill_run_list(&SkillRunListParams {
                run_ids: vec!["run-test".into()],
            })
            .unwrap();
        let installation = &result.installations[0];
        assert_eq!(installation.status, "installed");
        assert_eq!(installation.message_id.as_deref(), Some("final"));
        let saved = server
            .skill_get(&contexthub_protocol::SkillGetParams {
                skill_id: installation.skill_id.clone().unwrap(),
            })
            .unwrap()
            .skill;
        assert!(saved.instructions.contains("```bash\n"));
        assert_eq!(saved.files.len(), 1);
        server
            .ai_state_replace(
                &AiStateReplaceParams {
                    expected_revision: 1,
                    state,
                },
                3,
            )
            .unwrap();
        server.process_pending_skills(4).unwrap();
        let retry = server
            .skill_run_retry(
                &SkillRunRetryParams {
                    run_id: "run-test".into(),
                },
                5,
            )
            .unwrap();
        assert_eq!(retry.skill_id, installation.skill_id);
        assert_eq!(
            server
                .skill_list(&SkillListParams {})
                .unwrap()
                .skills
                .iter()
                .filter(|skill| skill.source == "user")
                .count(),
            1
        );
        drop(server);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn ordinary_replies_do_not_show_skill_installation_while_queued() {
        let root = root();
        let mut server = services(&root);
        server
            .ai_state_replace(
                &AiStateReplaceParams {
                    expected_revision: 0,
                    state: state("普通回答，无技能交付。"),
                },
                1,
            )
            .unwrap();
        let record = server
            .persistence()
            .unwrap()
            .skill_installation("run-test")
            .unwrap()
            .unwrap();
        assert_eq!(record.status, "pending");
        let mut native_source: Value = serde_json::from_str(&record.source_json).unwrap();
        native_source["skillDelivery"] = structured_delivery("普通回答，无技能交付。");
        native_source["message"]
            .as_object_mut()
            .unwrap()
            .remove("content");
        let native_record = SkillInstallationRecord {
            source_json: native_source.to_string(),
            ..record
        };
        assert_eq!(
            server
                .skill_installation_view(&native_record)
                .unwrap()
                .status,
            "ignored"
        );
        assert!(
            server
                .skill_run_list(&SkillRunListParams {
                    run_ids: vec!["run-test".into()]
                })
                .unwrap()
                .installations
                .is_empty()
        );
        server.process_pending_skills(2).unwrap();
        assert!(
            server
                .skill_run_list(&SkillRunListParams {
                    run_ids: vec!["run-test".into()]
                })
                .unwrap()
                .installations
                .is_empty()
        );
        drop(server);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn queued_skill_delivery_keeps_pending_and_invalid_delivery_is_explicit() {
        let root = root();
        let mut server = services(&root);
        server
            .ai_state_replace(
                &AiStateReplaceParams {
                    expected_revision: 0,
                    state: state(&content("真实交付")),
                },
                1,
            )
            .unwrap();
        let result = server
            .skill_run_list(&SkillRunListParams {
                run_ids: vec!["run-test".into()],
            })
            .unwrap();
        assert_eq!(result.installations[0].status, "pending");
        assert!(!result.installations[0].retryable);
        let mut record = server
            .persistence()
            .unwrap()
            .skill_installation("run-test")
            .unwrap()
            .unwrap();
        let mut source: Value = serde_json::from_str(&record.source_json).unwrap();
        source["skillDelivery"] = structured_delivery("```cilai-skill\n{");
        record.source_json = source.to_string();
        let invalid = server.skill_installation_view(&record).unwrap();
        assert_eq!(invalid.status, "invalid");
        assert_eq!(
            invalid.error_reason.as_deref(),
            Some("skill_delivery_incomplete")
        );
        drop(server);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn invalid_package_does_not_write_files_and_failure_is_durable() {
        let root = root();
        let mut server = services(&root);
        let text = content("非法路径").replace("references/example.md", "references/../secret.md");
        server
            .ai_state_replace(
                &AiStateReplaceParams {
                    expected_revision: 0,
                    state: state(&text),
                },
                1,
            )
            .unwrap();
        server.process_pending_skills(2).unwrap();
        drop(server);
        let mut server = services(&root);
        let result = server
            .skill_run_list(&SkillRunListParams {
                run_ids: vec!["run-test".into()],
            })
            .unwrap();
        assert_eq!(result.installations[0].status, "invalid");
        assert_eq!(
            result.installations[0].error_reason.as_deref(),
            Some("skill_file_path_invalid")
        );
        assert!(!result.installations[0].retryable);
        assert!(
            server
                .skill_list(&SkillListParams {})
                .unwrap()
                .skills
                .iter()
                .all(|skill| skill.source != "user")
        );
        assert!(!root.join("skills/user").exists());
        assert!(
            server
                .persistence()
                .unwrap()
                .pending_skill_installations()
                .unwrap()
                .is_empty()
        );
        drop(server);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn deleted_skill_is_not_reinstalled_by_replay() {
        let root = root();
        let mut server = services(&root);
        server
            .ai_state_replace(
                &AiStateReplaceParams {
                    expected_revision: 0,
                    state: state(&content("删除测试")),
                },
                1,
            )
            .unwrap();
        server.process_pending_skills(2).unwrap();
        let skill = server
            .skill_list(&SkillListParams {})
            .unwrap()
            .skills
            .remove(0);
        server
            .skill_delete(
                &contexthub_protocol::SkillDeleteParams {
                    skill_id: skill.skill_id,
                    expected_revision: skill.state_revision,
                    client_operation_id: "delete-test".into(),
                    request_fingerprint: "b".repeat(64),
                },
                3,
            )
            .unwrap();
        let result = server
            .skill_run_retry(
                &SkillRunRetryParams {
                    run_id: "run-test".into(),
                },
                4,
            )
            .unwrap();
        assert_eq!(result.status, "deleted");
        assert!(
            server
                .skill_list(&SkillListParams {})
                .unwrap()
                .skills
                .iter()
                .all(|skill| skill.source != "user")
        );
        drop(server);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn save_failure_is_not_retried_until_requested() {
        let root = root();
        let mut server = services(&root);
        fs::write(root.join("skills"), "occupied").unwrap();
        server
            .ai_state_replace(
                &AiStateReplaceParams {
                    expected_revision: 0,
                    state: state(&content("重试测试")),
                },
                1,
            )
            .unwrap();
        server.process_pending_skills(2).unwrap();
        let result = server
            .skill_run_list(&SkillRunListParams {
                run_ids: vec!["run-test".into()],
            })
            .unwrap();
        assert_eq!(result.installations[0].status, "failed");
        assert!(result.installations[0].retryable);
        fs::remove_file(root.join("skills")).unwrap();
        server.process_pending_skills(3).unwrap();
        assert!(
            server
                .skill_list(&SkillListParams {})
                .unwrap()
                .skills
                .iter()
                .all(|skill| skill.source != "user")
        );
        let retried = server
            .skill_run_retry(
                &SkillRunRetryParams {
                    run_id: "run-test".into(),
                },
                4,
            )
            .unwrap();
        assert_eq!(retried.status, "installed");
        assert_eq!(
            server
                .skill_list(&SkillListParams {})
                .unwrap()
                .skills
                .iter()
                .filter(|skill| skill.source == "user")
                .count(),
            1
        );
        drop(server);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn historical_delivery_requires_explicit_retry_and_missing_runs_cannot_install() {
        let root = root();
        let mut server = services(&root);
        let state = state(&content("历史交付"));
        server
            .ai_state_replace(
                &AiStateReplaceParams {
                    expected_revision: 0,
                    state: state.clone(),
                },
                1,
            )
            .unwrap();
        let connection = rusqlite::Connection::open(root.join("contexthub.db")).unwrap();
        connection
            .execute("UPDATE ai_skill_installations SET status='historical'", [])
            .unwrap();
        server.process_pending_skills(2).unwrap();
        assert!(
            server
                .skill_list(&SkillListParams {})
                .unwrap()
                .skills
                .iter()
                .all(|skill| skill.source != "user")
        );
        let projected = server
            .skill_run_list(&SkillRunListParams {
                run_ids: vec!["run-test".into()],
            })
            .unwrap();
        assert_eq!(projected.installations[0].status, "historical");
        assert!(projected.installations[0].retryable);
        let result = server
            .skill_run_retry(
                &SkillRunRetryParams {
                    run_id: "run-test".into(),
                },
                3,
            )
            .unwrap();
        assert_eq!(result.status, "installed");
        let mut deleted = state;
        deleted["runs"] = json!([]);
        deleted["messages"] = json!([]);
        server
            .ai_state_replace(
                &AiStateReplaceParams {
                    expected_revision: 1,
                    state: deleted,
                },
                4,
            )
            .unwrap();
        connection
            .execute(
                "UPDATE ai_skill_installations SET status='pending',result_json=NULL",
                [],
            )
            .unwrap();
        server.process_pending_skills(5).unwrap();
        assert_eq!(
            server
                .persistence()
                .unwrap()
                .skill_installation("run-test")
                .unwrap()
                .unwrap()
                .status,
            "ignored"
        );
        drop(connection);
        drop(server);
        fs::remove_dir_all(root).unwrap();
    }
}

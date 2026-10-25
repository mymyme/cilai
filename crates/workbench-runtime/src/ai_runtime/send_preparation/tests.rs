use super::{LockedPreparation, RunPolicy, normalize_request};
use crate::ai_runtime::{
    AiRuntimeManager, MAX_MESSAGE_CHARS, MessageAttachment, MessageRegenerationEdit,
    MessageSendRequest, ModelInfo, PendingAttachment, lock,
};
use serde_json::json;

fn request() -> MessageSendRequest {
    serde_json::from_value(json!({
        "conversationId":"conversation_1","content":" 用户要求 ","executionTarget":"model",
        "permissionMode":"request","clientMessageId":"input"
    }))
    .unwrap()
}

fn model() -> ModelInfo {
    crate::ai_runtime::system_model_catalog().remove(0)
}

fn attachment(media_type: &str) -> MessageAttachment {
    MessageAttachment {
        attachment_id: "attachment".into(),
        file_name: "input.png".into(),
        mime_type: "image/png".into(),
        media_type: media_type.into(),
        size_bytes: 1,
        local_path: "/private/input.png".into(),
    }
}

#[test]
fn normalization_removes_client_material_paths_before_admission() {
    let mut request = request();
    request.referenced_materials = serde_json::from_value(json!([{
        "localFileId":"file","relativePath":"input.md","fileRevision":1,
        "resolvedPath":"/private/untrusted","cloudReference":null
    }]))
    .unwrap();
    normalize_request(&mut request, false).unwrap();
    assert!(request.referenced_materials[0].resolved_path.is_none());
    assert!(request.referenced_materials[0].cloud_reference.is_none());
    request.content = "中".repeat(MAX_MESSAGE_CHARS + 1);
    assert_eq!(
        normalize_request(&mut request, false).unwrap_err(),
        "ai_message_too_large"
    );
    request.content = " ".into();
    assert_eq!(
        normalize_request(&mut request, true).unwrap_err(),
        "ai_message_required"
    );
}

#[test]
fn attachments_are_checked_against_the_current_locked_upload_state() {
    let manager = AiRuntimeManager::new();
    let mut request = request();
    request.attachment_ids = vec!["attachment".into()];
    let model = model();
    let mut state = lock(&manager.inner);
    state.pending_attachments.insert(
        "attachment".into(),
        PendingAttachment {
            attachment: attachment("image"),
            received_bytes: 1,
            complete: false,
        },
    );
    assert_eq!(
        LockedPreparation::new(&state, &request, &model)
            .requested_attachments(false)
            .unwrap_err(),
        "ai_attachment_incomplete"
    );
    state
        .pending_attachments
        .get_mut("attachment")
        .unwrap()
        .complete = true;
    let prepared = LockedPreparation::new(&state, &request, &model)
        .requested_attachments(false)
        .unwrap();
    assert_eq!(prepared[0].attachment_id, "attachment");
    state.pending_attachments.remove("attachment");
    assert_eq!(
        LockedPreparation::new(&state, &request, &model)
            .requested_attachments(false)
            .unwrap_err(),
        "ai_attachment_not_found"
    );
    assert!(
        LockedPreparation::new(&state, &request, &model)
            .requested_attachments(true)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn history_checks_modalities_in_the_actual_regeneration_prefix() {
    let manager = AiRuntimeManager::new();
    let conversation = crate::ai_runtime::tests::fixture_conversation("agent");
    let mut first = crate::ai_runtime::tests::fixture_message(&conversation.conversation_id);
    first.message_id = "first".into();
    let mut later = first.clone();
    later.message_id = "later".into();
    later.attachments = vec![attachment("image")];
    let request = request();
    let model = model();
    let mut state = lock(&manager.inner);
    state.persisted.messages = vec![first.clone(), later];
    let mut edited = first.clone();
    edited.content = "重新生成的要求".into();
    let preparation = LockedPreparation::new(&state, &request, &model);
    let prefix = preparation
        .history(&conversation, &edited, Some(&first))
        .unwrap();
    assert_eq!(prefix.len(), 1);
    assert_eq!(prefix[0].content, "重新生成的要求");
    assert_eq!(
        preparation
            .history(&conversation, &edited, None)
            .unwrap_err(),
        "model_history_incompatible"
    );
    assert_eq!(state.persisted.messages[0].content, first.content);
}

#[test]
fn retry_inputs_are_immutable_until_the_manager_accepts_the_run() {
    let manager = AiRuntimeManager::new();
    let conversation = crate::ai_runtime::tests::fixture_conversation("agent");
    let mut original = crate::ai_runtime::tests::fixture_message(&conversation.conversation_id);
    original.message_id = "original".into();
    original.run_id = Some("previous".into());
    let request = request();
    let model = model();
    let mut state = lock(&manager.inner);
    state.persisted.messages.push(original.clone());
    state.persisted.runs.push(
        serde_json::from_value(json!({
            "runId":"previous","conversationId":conversation.conversation_id,"agentId":"agent",
            "inputMessageId":"original","provider":"codex","status":"failed","startedAt":1
        }))
        .unwrap(),
    );
    let edit = MessageRegenerationEdit {
        message_id: "original".into(),
        content: " 修改要求 ".into(),
    };
    let input = LockedPreparation::new(&state, &request, &model)
        .input_message(&conversation, Some("previous"), Some(&edit), Vec::new(), 10)
        .unwrap();
    assert_eq!(input.content, "修改要求");
    assert_eq!(input.message_id, "original");
    assert_eq!(input.run_id.as_deref(), Some("previous"));
    assert_eq!(state.persisted.messages[0], original);
    assert_eq!(state.persisted.runs.len(), 1);
}

#[test]
fn plan_mode_cannot_inherit_write_capabilities_from_full_access() {
    let conversation = crate::ai_runtime::tests::fixture_conversation("agent");
    let mut request = request();
    request.permission_mode = "full-access".into();
    let full = RunPolicy::prepare(&request, &conversation).unwrap();
    assert_eq!(full.approval_mode, "full-access");
    assert_eq!(full.sandbox_mode, "danger-full-access");
    assert!(full.run_tools.iter().any(|tool| tool == "write-files"));
    request.planning = true;
    let plan = RunPolicy::prepare(&request, &conversation).unwrap();
    assert_eq!(plan.sandbox_mode, "read-only");
    assert!(!plan.run_tools.iter().any(|tool| tool == "write-files"));
    assert!(plan.prompt.contains("不得执行文件修改"));
    assert!(
        !plan
            .run_tools
            .iter()
            .any(|tool| tool == "application-read-action")
    );
}

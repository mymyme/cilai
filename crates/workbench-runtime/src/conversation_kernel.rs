use crate::codex_harness::CodexHarnessRequest;
use crate::persistent_threads::{PersistentThread, ThreadJournal};
use crate::supervisor::AppServerSupervisor;
use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConversationKind {
    Workbench,
    ApplicationPage,
    ApplicationTask,
}

impl ConversationKind {
    pub(crate) fn allows_global_tools(self) -> bool {
        self == Self::Workbench
    }

    pub(crate) fn harness_home(self, data_dir: &Path) -> PathBuf {
        match self {
            Self::Workbench => crate::codex_upgrade::official_home(&data_dir.join("cilai-harness")),
            Self::ApplicationPage | Self::ApplicationTask => {
                crate::codex_upgrade::official_home(&data_dir.join("cilai-application-harness"))
            }
        }
    }

    pub(crate) fn validate(self, request: &CodexHarnessRequest) -> Result<(), String> {
        if self == Self::Workbench {
            return Ok(());
        }
        if let Some(schema) = request
            .application_result
            .as_ref()
            .and_then(|task| task.result_schema.as_ref())
        {
            crate::application_result::validate_result_schema(schema)?;
        }
        if request.memory_preferences.enabled
            || (request.personal_tools.is_some() && !self.allows_personal_page_tools(request))
            || request.user_mcp.is_some()
            || request.classification.is_some()
            || request.sandbox_mode != "read-only"
            || !request.application_mcp.as_ref().is_some_and(|scope| {
                (scope.restrict_to_applications
                    && !scope.report_review
                    && scope.application_ids.len() == 1
                    && scope.application_ids[0] == scope.application_id)
                    || (self == Self::ApplicationPage
                        && scope.report_review
                        && request.application_action.is_none()
                        && request.application_write.is_none()
                        && !scope.restrict_to_applications
                        && scope.write_tools.is_empty()
                        && scope.application_ids.contains(&scope.application_id))
            })
        {
            return Err("application_execution_scope_invalid".to_owned());
        }
        if self == Self::ApplicationTask
            && (request.application_action.is_some()
                || request.application_write.is_some()
                || request.resume_thread_id.is_some())
        {
            return Err("application_task_scope_invalid".to_owned());
        }
        Ok(())
    }

    fn allows_personal_page_tools(self, request: &CodexHarnessRequest) -> bool {
        self == Self::ApplicationPage
            && request.application_adapter.is_none()
            && request.execution.as_ref().is_some_and(|execution| {
                execution.foreground.lease.snapshot.purpose
                    == contexthub_protocol::ModelPurpose::PageAi
                    && execution.foreground.lease.snapshot.runtime_kind
                        == contexthub_protocol::ModelRuntimeKind::Application
                    && execution.foreground.lease.snapshot.binding_id.is_some()
            })
            && request.application_mcp.as_ref().is_some_and(|scope| {
                scope.restrict_to_applications
                    && !scope.report_review
                    && scope.application_ids == [scope.application_id.clone()]
                    && scope.read_tools.is_empty()
                    && scope.write_tools.is_empty()
                    && scope.tool_names.is_empty()
            })
    }
}

pub(crate) struct ThreadInput {
    pub application_scope: Option<contexthub_protocol::AiThreadApplicationScope>,
    pub conversation_id: String,
    pub run_id: String,
    pub message_id: String,
    pub client_message_id: String,
    pub content_digest: String,
}

pub(crate) fn thread_plan(
    server: AppServerSupervisor,
    input: ThreadInput,
    native_thread_id: Option<String>,
    fork_before_turn_id: Option<String>,
) -> PersistentThread {
    let name = if input.application_scope.is_some() {
        "应用临时对话"
    } else {
        "新对话"
    }
    .to_owned();
    PersistentThread {
        journal: ThreadJournal {
            writer_process: Value::Null,
            runtime_scope: None,
            model_execution_lease_id: None,
            history_consent_confirmed: false,
            application_scope: input.application_scope,
            server,
            conversation_id: input.conversation_id,
            run_id: input.run_id,
            owner_instance_id: crate::persistent_threads::owner_instance_id(),
        },
        native_thread_id,
        fork_before_turn_id,
        workbench_history_fork: false,
        cross_runtime_fork: false,
        workbench_history: Vec::new(),
        name,
        history: Vec::new(),
        message_id: input.message_id,
        client_message_id: input.client_message_id,
        content_digest: input.content_digest,
        writer_process: Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_formal_conversations_allow_global_tools() {
        assert!(ConversationKind::Workbench.allows_global_tools());
        assert!(!ConversationKind::ApplicationPage.allows_global_tools());
        assert!(!ConversationKind::ApplicationTask.allows_global_tools());
    }

    #[test]
    fn cilai_home_migration_recovers_the_same_real_native_threads() {
        let _guard = crate::supervisor::app_server_test_guard();
        let root = crate::ai_runtime::model_test_fixture::fixture_root("cilai-native-migration")
            .canonicalize()
            .unwrap();
        let profile = root.join("test");
        // Match normal startup: the profile is private before native components
        // create their inherited runtime directories.
        contexthub_platform_adapter::security::ensure_restricted_directory(&profile).unwrap();
        let workspace = root.join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let executable = crate::supervisor::codex_harness_test_binary();
        let mut threads = Vec::new();
        for (old, new) in [
            ("codex-harness", "cilai-harness"),
            ("application-harness", "cilai-application-harness"),
        ] {
            let home = profile
                .join("ai-runtime")
                .join(old)
                .join("official-runtime");
            let service =
                crate::codex_threads::CodexThreadService::new(executable.clone(), home.clone());
            let created = service.request("thread/start", &json!({"cwd":workspace,"approvalPolicy":"never","sandbox":"read-only","ephemeral":false})).unwrap();
            let id = created["thread"]["id"].as_str().unwrap().to_owned();
            service.request("thread/inject_items", &json!({"threadId":id,"items":[{"type":"message","role":"user","content":[{"type":"input_text","text":"保留迁移前问题"}]}]})).unwrap();
            service
                .request(
                    "thread/name/set",
                    &json!({"threadId":id,"name":"迁移前标题"}),
                )
                .unwrap();
            let archived = old == "codex-harness";
            if archived {
                service
                    .request("thread/archive", &json!({"threadId":id}))
                    .unwrap();
            }
            let history = service.turns(&id).unwrap();
            service.shutdown();
            let receipt = json!({"status":"verified","targetVersion":"0.163.0","homeDigest":crate::persistent_threads::digest(&json!(home.canonicalize().unwrap())),"providerAliases":[]});
            contexthub_platform_adapter::security::create_new_restricted_file(
                &home.join("cilai-public-history-migration.json"),
            )
            .unwrap();
            std::fs::write(
                home.join("cilai-public-history-migration.json"),
                receipt.to_string(),
            )
            .unwrap();
            threads.push((old, new, id, archived, history));
        }
        let server = AppServerSupervisor::new();
        server
            .start(
                &crate::supervisor::app_server_test_binary(),
                "test",
                &profile,
                &profile.join("run"),
            )
            .unwrap_or_else(|error| panic!("{error}; stderr={:?}", server.status().stderr_tail));
        for (old, new, id, archived, before) in threads {
            assert!(!profile.join("ai-runtime").join(old).exists());
            let service = crate::codex_threads::CodexThreadService::new(
                executable.clone(),
                profile
                    .join("ai-runtime")
                    .join(new)
                    .join("official-runtime"),
            );
            let catalog = service.catalog_for_threads([id.as_str()]).unwrap();
            assert_eq!(catalog[&id]["name"], "迁移前标题");
            assert_eq!(catalog[&id]["archived"], archived);
            let history = service.turns(&id).unwrap();
            assert_eq!(history, before);
            if archived {
                service
                    .request("thread/unarchive", &json!({"threadId":id}))
                    .unwrap();
            }
            let resumed = service.request("thread/resume", &json!({"threadId":id,"cwd":workspace,"approvalPolicy":"never","sandbox":"read-only"})).unwrap();
            assert_eq!(resumed["thread"]["id"], id);
            service.shutdown();
            assert!(
                profile
                    .join("ai-runtime/cilai-home-migrations-v1")
                    .join(new)
                    .join("source")
                    .is_dir()
            );
        }
        server.shutdown().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn temporary_threads_cannot_be_scanned_by_workbench_memory_or_search() {
        let root = Path::new("/profile/ai-runtime");
        assert_ne!(
            ConversationKind::Workbench.harness_home(root),
            ConversationKind::ApplicationPage.harness_home(root)
        );
        assert_ne!(
            ConversationKind::Workbench.harness_home(root),
            ConversationKind::ApplicationTask.harness_home(root)
        );
    }

    #[test]
    fn native_temporary_threads_survive_reopen_without_entering_the_formal_catalog() {
        let root = std::env::temp_dir().join(format!("kernel-native-{}", uuid::Uuid::new_v4()));
        let workspace = root.join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let executable = crate::supervisor::codex_harness_test_binary();
        let temporary = crate::codex_threads::CodexThreadService::new(
            executable.clone(),
            ConversationKind::ApplicationPage.harness_home(&root),
        );
        let created = temporary.request("thread/start", &json!({"cwd":workspace,"approvalPolicy":"never","sandbox":"read-only","ephemeral":false})).unwrap();
        let id = created["thread"]["id"].as_str().unwrap();
        temporary.request("thread/inject_items", &json!({"threadId":id,"items":[{"type":"message","role":"user","content":[{"type":"input_text","text":"仅页面可见"}]}]})).unwrap();
        temporary
            .request(
                "thread/name/set",
                &json!({"threadId":id,"name":"仅页面可见"}),
            )
            .unwrap();
        temporary.shutdown();
        assert_eq!(
            temporary
                .request("thread/read", &json!({"threadId":id,"includeTurns":false}))
                .unwrap()["thread"]["name"],
            "仅页面可见"
        );
        let formal = crate::codex_threads::CodexThreadService::new(
            executable,
            ConversationKind::Workbench.harness_home(&root),
        );
        assert!(!formal.catalog().unwrap().contains_key(id));
        assert!(
            formal
                .request("thread/read", &json!({"threadId":id,"includeTurns":false}))
                .is_err()
        );
        formal.shutdown();
        temporary.shutdown();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn application_history_uses_the_same_authoritative_journal_without_a_formal_conversation() {
        let _guard = crate::supervisor::app_server_test_guard();
        let root = std::env::temp_dir().join(format!("kernel-journal-{}", uuid::Uuid::new_v4()));
        let server = AppServerSupervisor::new();
        let data = root.join("test");
        server
            .start(
                &crate::supervisor::app_server_test_binary(),
                "test",
                &data,
                &data.join("run"),
            )
            .unwrap_or_else(|error| panic!("{error}: {:?}", server.status()));
        for round in 1..=20 {
            let state = server
                .request_result(
                    "aiThread/get",
                    &json!({"conversationId":"application:page"}),
                )
                .unwrap();
            let native = crate::persistent_threads::validate_binding(
                &state,
                &crate::persistent_threads::owner_instance_id(),
            )
            .unwrap();
            assert_eq!(native.as_deref(), (round > 1).then_some("native-page"));
            let plan = thread_plan(
                server.clone(),
                ThreadInput {
                    application_scope: Some(contexthub_protocol::AiThreadApplicationScope {
                        application_id: "app".into(),
                        session_kind:
                            contexthub_protocol::AiApplicationSessionKind::ApplicationPage,
                    }),
                    conversation_id: "application:page".into(),
                    run_id: format!("app_run_{round}"),
                    message_id: format!("m{round}"),
                    client_message_id: format!("m{round}"),
                    content_digest: format!("digest{round}"),
                },
                native,
                None,
            );
            plan.journal
                .prepare(
                    "thread",
                    if round == 1 { "create" } else { "resume" },
                    json!({}),
                )
                .unwrap();
            plan.journal
                .record("thread", "dispatch", Value::Null)
                .unwrap();
            plan.journal
                .record(
                    "thread",
                    "acknowledge",
                    json!({"nativeThreadId":"native-page"}),
                )
                .unwrap();
            plan.journal
                .prepare_input(
                    &format!("m{round}"),
                    &format!("m{round}"),
                    &format!("digest{round}"),
                    false,
                    "原文",
                )
                .unwrap();
            plan.journal
                .accept_input(&format!("m{round}"), "native-page", &format!("t{round}"))
                .unwrap();
            plan.journal.release().unwrap();
        }
        server.shutdown().unwrap();
        server
            .start(
                &crate::supervisor::app_server_test_binary(),
                "test",
                &data,
                &data.join("run"),
            )
            .unwrap();
        let state = server
            .request_result(
                "aiThread/get",
                &json!({"conversationId":"application:page"}),
            )
            .unwrap();
        assert_eq!(state["messages"].as_array().unwrap().len(), 20);
        assert_eq!(state["binding"]["nativeThreadId"], "native-page");
        let other = server
            .request_result(
                "aiThread/get",
                &json!({"conversationId":"application:other"}),
            )
            .unwrap();
        assert!(other["binding"].is_null());
        server.shutdown().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}

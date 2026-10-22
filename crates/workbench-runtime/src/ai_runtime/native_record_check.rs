use super::{
    AiRuntimeManager, LocalOperationGuard, codex_threads, lock, service_for_run, unix_epoch_millis,
};
use contexthub_protocol::{PersonalWriterLease, PersonalWriterMode, PersonalWriterState};
use serde_json::{Value, json};
use std::{io::Write as _, path::Path};

struct ReceiptWriter {
    server: crate::supervisor::AppServerSupervisor,
    lease: PersonalWriterLease,
}

impl ReceiptWriter {
    fn prepare(server: &crate::supervisor::AppServerSupervisor, run: &str) -> Result<Self, String> {
        let owner = crate::model_connections::execution_owner()?;
        let lease: PersonalWriterLease = serde_json::from_value(server.request_result(
            "profileWriter/prepare", &json!({
                "clientOperationId":format!("native-check-{}",uuid::Uuid::new_v4().simple()),
                "kind":"local_operation","resourceId":crate::persistent_threads::digest(&json!({"nativeCheck":run})),
                "resourceRevision":"1","owner":owner,"launchProtocol":1,"processMode":"remote_request"
            }),
        )?).map_err(|_| "native_record_check_writer_unconfirmed")?;
        if lease.state != PersonalWriterState::Active
            || lease.process_mode != PersonalWriterMode::RemoteRequest
            || lease.owner != owner
        {
            return Err("native_record_check_writer_unconfirmed".into());
        }
        Ok(Self {
            server: server.clone(),
            lease,
        })
    }

    fn finish(self) -> Result<(), String> {
        let released: PersonalWriterLease = serde_json::from_value(self.server.request_result(
            "profileWriter/release",
            &json!({"leaseId":self.lease.lease_id,"owner":self.lease.owner,
                "localTransportFinished":true,"remoteOutcomeUnconfirmed":false}),
        )?)
        .map_err(|_| "native_record_check_writer_unconfirmed")?;
        if released.state != PersonalWriterState::Released {
            return Err("native_record_check_writer_unconfirmed".into());
        }
        Ok(())
    }
}

fn write_receipt(path: &Path, receipt: &Value) -> Result<(), String> {
    let mut file = contexthub_platform_adapter::security::create_new_restricted_file(path)
        .map_err(|_| "native_record_check_receipt_write_unconfirmed")?;
    file.write_all(receipt.to_string().as_bytes())
        .and_then(|()| file.sync_all())
        .and_then(|()| {
            contexthub_platform_adapter::filesystem::sync_directory(
                path.parent()
                    .ok_or_else(|| std::io::Error::other("native_record_check_path_invalid"))?,
            )
        })
        .map_err(|_| "native_record_check_receipt_write_unconfirmed".into())
}

struct Verification {
    service: std::sync::Arc<crate::codex_threads::CodexThreadService>,
    server: crate::supervisor::AppServerSupervisor,
    data_dir: std::path::PathBuf,
    run: super::AgentRun,
    users: Vec<(String, String)>,
    reply: Option<String>,
    _operation: LocalOperationGuard,
    writer: ReceiptWriter,
}

impl Verification {
    fn prepare(manager: &AiRuntimeManager, run_id: &str) -> Option<Self> {
        let mut state = lock(&manager.inner);
        if state.shutting_down || state.maintenance_fence.is_some() {
            return None;
        }
        let run = state
            .persisted
            .runs
            .iter()
            .find(|run| run.run_id == run_id)?;
        if run.status != "completed" {
            return None;
        }
        let (Ok(service), Some(server), Some(data_dir)) = (
            service_for_run(&state, run),
            state.server.clone(),
            state.data_dir.clone(),
        ) else {
            eprintln!("codex_thread_record_check run={run_id} outcome=unavailable");
            return None;
        };
        let users = state
            .persisted
            .messages
            .iter()
            .filter(|message| {
                message.role == "user"
                    && (message.message_id == run.input_message_id
                        || message.run_id.as_deref() == Some(run_id))
            })
            .map(|message| (message.message_id.clone(), message.content.clone()))
            .collect();
        let reply = state
            .persisted
            .messages
            .iter()
            .rev()
            .find(|message| {
                message.role == "assistant" && message.run_id.as_deref() == Some(run_id)
            })
            .map(|message| message.content.clone());
        let run = run.clone();
        let writer = match ReceiptWriter::prepare(&server, run_id) {
            Ok(writer) => writer,
            Err(error) => {
                eprintln!(
                    "codex_thread_record_check run={run_id} outcome=writer_unavailable error={error}"
                );
                return None;
            }
        };
        state.local_operations += 1;
        Some(Self {
            service,
            server,
            data_dir,
            run,
            users,
            reply,
            _operation: LocalOperationGuard {
                manager: manager.clone(),
            },
            writer,
        })
    }

    fn execute(self, manager: &AiRuntimeManager) {
        let outcome = (|| {
            let thread_id = self.run.session_id.as_deref().ok_or("thread_id_missing")?;
            let journal = self.server.request_result(
                "aiThread/get",
                &json!({"conversationId":self.run.conversation_id}),
            )?;
            let turns = self.service.turns(thread_id)?;
            codex_threads::compare_run(
                &turns,
                &journal,
                &self.run.run_id,
                thread_id,
                &self.users,
                self.reply.as_deref(),
                self.run.classification_nonce.as_deref(),
            )
        })();
        let stopped = self.service.shutdown_checked();
        let exited = stopped.is_ok();
        if !exited {
            lock(&manager.inner).harness_retirement_error =
                Some("native_record_check_exit_unconfirmed".into());
        }
        let outcome = outcome.and(stopped);
        let receipt = json!({"version":1,"runId":self.run.run_id,"threadId":self.run.session_id,
            "status":if outcome.is_ok(){"matched"}else{"failed"},"reason":outcome.err(),
            "userMessageCount":self.users.len(),"checkedAt":unix_epoch_millis().ok()});
        let path = self
            .data_dir
            .join("runs")
            .join(format!("{}.thread-check.json", self.run.run_id));
        let state = lock(&manager.inner);
        if state.data_dir.as_ref() != Some(&self.data_dir)
            || !state
                .persisted
                .runs
                .iter()
                .any(|item| item.run_id == self.run.run_id)
        {
            drop(state);
            lock(&manager.inner).harness_retirement_error =
                Some("native_record_check_profile_changed".into());
            return;
        }
        if let Err(error) = write_receipt(&path, &receipt) {
            drop(state);
            lock(&manager.inner).harness_retirement_error = Some(error.clone());
            eprintln!(
                "codex_thread_record_check run={} outcome=receipt_write_failed error={error}",
                self.run.run_id
            );
            return;
        }
        if receipt["status"] != "matched" {
            eprintln!("codex_thread_record_check {receipt}");
        }
        drop(state);
        if exited && let Err(error) = self.writer.finish() {
            lock(&manager.inner).harness_retirement_error = Some(error);
        }
    }
}

impl AiRuntimeManager {
    pub(super) fn verify_thread_record(&self, run_id: &str) {
        let Some(verification) = Verification::prepare(self, run_id) else {
            return;
        };
        let manager = self.clone();
        if let Err(error) = std::thread::Builder::new()
            .name("codex-thread-verify".to_owned())
            .spawn(move || verification.execute(&manager))
        {
            lock(&self.inner).harness_retirement_error =
                Some("native_record_check_spawn_unconfirmed".into());
            eprintln!("codex_thread_record_check run={run_id} outcome=spawn_failed error={error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai_runtime::tests::AutomationTestRuntime;
    use crate::supervisor::{AppServerConnectionMode, AppServerSupervisor};

    #[test]
    fn another_client_cannot_quiesce_before_the_real_receipt_is_synced_and_its_writer_is_released()
    {
        let _guard = crate::supervisor::app_server_test_guard();
        let runtime = AutomationTestRuntime::new();
        let peer = AppServerSupervisor::new();
        peer.start(
            &crate::supervisor::app_server_test_binary(),
            "test",
            &runtime.root,
            &runtime.root.join("run"),
        )
        .unwrap();
        assert_eq!(
            peer.status().connection_mode,
            Some(AppServerConnectionMode::Attached)
        );
        let writer = ReceiptWriter::prepare(&runtime.server, "inflight-native-check").unwrap();
        let lease_id = writer.lease.lease_id.clone();
        let operation = peer.request_result("personalData/backupPrepare", &json!({
            "selection":{"includeWorkspaceFiles":[]},"clientOperationId":"native-check-maintenance"
        })).unwrap();
        let owner = crate::model_connections::execution_owner().unwrap();
        let fence = peer.request_result("personalMaintenance/acquire", &json!({
            "operationId":operation["operationId"],"previewDigest":operation["previewDigest"],"owner":owner
        })).unwrap();
        let params = json!({"fenceId":fence["fenceId"],"owner":owner});
        let drain = || {
            peer.request_result("personalMaintenance/drainStatus", &params)
                .unwrap()
        };
        let active = |status: &Value| {
            status["activeWriters"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["leaseId"] == lease_id)
        };
        assert!(active(&drain()));
        assert!(
            ReceiptWriter::prepare(&runtime.server, "late-native-check")
                .err()
                .unwrap()
                .contains("personal_data_maintenance_active")
        );
        let path = runtime
            .root
            .join("ai-runtime/runs/native-check.fixture.json");
        let receipt = json!({"status":"matched","source":"isolated-test"});
        write_receipt(&path, &receipt).unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&std::fs::read(&path).unwrap()).unwrap(),
            receipt
        );
        assert!(active(&drain()));
        writer.finish().unwrap();
        assert!(!active(&drain()));
        peer.request_result("personalMaintenance/release", &params)
            .unwrap();
        peer.shutdown().unwrap();
    }

    #[test]
    fn a_receipt_cannot_overwrite_an_existing_or_unconfirmed_result() {
        let root = crate::ai_runtime::model_test_fixture::fixture_root("native-check-receipt");
        let path = root.join("receipt.json");
        let original = json!({"status":"failed"});
        write_receipt(&path, &original).unwrap();
        assert!(write_receipt(&path, &json!({"status":"matched"})).is_err());
        assert_eq!(
            serde_json::from_slice::<Value>(&std::fs::read(&path).unwrap()).unwrap(),
            original
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}

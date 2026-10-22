#![cfg(test)]
#![cfg(target_os = "macos")]

use super::*;
use crate::ai_runtime::model_test_fixture::{fixture_root, profile_data_root};
use crate::supervisor::AppServerSupervisor;
use std::fs;

struct Probe {
    root: PathBuf,
    manager: AiRuntimeManager,
    server: AppServerSupervisor,
    runtime: ConnectionRuntime,
    connection: ModelConnection,
    change: RootAuthChange,
    home: PathBuf,
}

impl Probe {
    fn new() -> Self {
        let root = fixture_root("official-guarded-file-auth");
        let profile = profile_data_root(&root);
        let server = AppServerSupervisor::new();
        server
            .start(
                &crate::supervisor::app_server_test_binary(),
                "test",
                &profile,
                &profile.join("run"),
            )
            .unwrap();
        let manager = AiRuntimeManager::new();
        manager.initialize(&profile, &server).unwrap();
        let connection = create_connection(&server);
        let change: RootAuthChange = serde_json::from_value(server.request_result("modelAuth/startPrepare", &json!({
            "operationId":"synthetic-file-auth-operation","connectionId":connection.connection_id,
            "expectedConfigRevision":connection.config_revision,"runtimeKind":"workbench",
            "owner":execution_owner().unwrap(),"launchProtocol":1
        })).unwrap()).unwrap();
        let scope = contexthub_protocol::model_runtime_home_scope(
            &connection.profile_id,
            &connection.connection_id,
            change.target_generation,
        );
        let home = scoped_home(
            &manager.gateway_data_dir().unwrap(),
            &scope,
            ModelRuntimeKind::Workbench,
        );
        fs::create_dir_all(&home).unwrap();
        Self {
            root,
            manager,
            server,
            runtime: ConnectionRuntime::default(),
            connection,
            change,
            home,
        }
    }

    fn service(&self) -> Arc<CodexThreadService> {
        self.runtime
            .auth_service(
                &self.manager,
                &self.connection,
                &self.change,
                ModelRuntimeKind::Workbench,
                self.change.target_generation,
            )
            .unwrap()
    }
}

impl Drop for Probe {
    fn drop(&mut self) {
        let stopped = self.runtime.shutdown().is_ok();
        self.manager.shutdown();
        let server_stopped = self.server.shutdown().is_ok();
        if !stopped || !server_stopped {
            eprintln!("synthetic_file_auth_cleanup_unconfirmed");
        }
        if !std::thread::panicking() && stopped && server_stopped {
            fs::remove_dir_all(&self.root).unwrap();
        } else {
            eprintln!("isolated_fixture_failure_evidence={}", self.root.display());
        }
    }
}

#[test]
#[ignore = "显式运行锁定官方运行件的合成本地文件认证；不属于真实 OAuth 或模型兼容测试"]
fn official_guarded_home_file_auth_survives_restart_and_logout() {
    let _guard = crate::supervisor::app_server_test_guard();
    let probe = Probe::new();
    let service = probe.service();
    let synthetic = format!("isolated-synthetic-{}", uuid::Uuid::new_v4().simple());
    let saved = service.request(
        "account/login/start",
        &json!({"type":"apiKey","apiKey":synthetic}),
    );
    if let Err(error) = &saved {
        eprintln!(
            "synthetic_file_auth_save_failed={}",
            error.replace(&synthetic, "[redacted]")
        );
    }
    assert!(saved.is_ok());
    service.shutdown_checked().unwrap();
    let restarted = probe.service();
    let read = restarted
        .request("account/read", &json!({"refreshToken":false}))
        .unwrap();
    assert_eq!(read["account"]["type"], "apiKey");
    contexthub_platform_adapter::security::verify_current_user_private_file(
        &probe.home.join("auth.json"),
    )
    .unwrap();
    assert!(
        fs::read(probe.home.join("auth.json"))
            .unwrap()
            .windows(synthetic.len())
            .any(|bytes| bytes == synthetic.as_bytes())
    );
    restarted.request("account/logout", &json!({})).unwrap();
    restarted.shutdown_checked().unwrap();
    let signed_out = probe.service();
    assert!(
        signed_out
            .request("account/read", &json!({"refreshToken":false}))
            .unwrap()["account"]
            .is_null()
    );
    signed_out.shutdown_checked().unwrap();
    step(
        &probe.manager,
        &probe.change,
        ModelRuntimeKind::Workbench,
        probe.change.target_generation,
        "signed_out",
        None,
    )
    .unwrap();
    probe
        .server
        .request_result(
            "modelAuth/cancel",
            &json!({"operationId":probe.change.operation_id}),
        )
        .unwrap();
    let recorded: RootAuthChange = serde_json::from_value(
        probe
            .server
            .request_result(
                "modelAuth/get",
                &json!({"operationId":probe.change.operation_id}),
            )
            .unwrap(),
    )
    .unwrap();
    assert_eq!(recorded.state, ModelAuthChangeState::Cancelled);
    assert_eq!(recorded.processes.len(), 3);
    processes_exited(
        &recorded,
        ModelRuntimeKind::Workbench,
        probe.change.target_generation,
    )
    .unwrap();
    assert!(!probe.home.join("auth.json").exists());
}

fn create_connection(server: &AppServerSupervisor) -> ModelConnection {
    let prepared = server.request_result("modelConnection/savePrepare", &json!({
            "operationId":"synthetic-file-auth-connection","connectionId":null,"expectedConfigRevision":null,
            "config":{"name":"独立合成凭据核验","kind":"chatgpt","baseUrl":null,"localRuntime":null,
                "authMode":"runtime_managed_chatgpt","enabled":true,
                "network":{"mode":"custom_proxy","proxyUrl":"http://127.0.0.1:9","proxySecretRef":null}},
            "credentialAction":"keep"
        })).unwrap();
    let connection: ModelConnection = serde_json::from_value(
        server
            .request_result(
                "modelConnection/saveCommit",
                &json!({
                    "operationId":prepared["operationId"],"credentialRef":null
                }),
            )
            .unwrap(),
    )
    .unwrap();
    connection
}

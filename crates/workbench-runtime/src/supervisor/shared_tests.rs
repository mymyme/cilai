use super::*;
use crate::ai_runtime::model_test_fixture::{fixture_root, profile_data_root};
use contexthub_platform_adapter::instance::process_start_fingerprint;

struct Fixture {
    root: PathBuf,
    profile: PathBuf,
    supervisors: Vec<AppServerSupervisor>,
    process: (u32, String),
}

impl Fixture {
    fn new() -> Self {
        let root = fixture_root("shared-app-server");
        let profile = profile_data_root(&root);
        let first = AppServerSupervisor::new();
        first
            .start(
                &app_server_test_binary(),
                "test",
                &profile,
                &profile.join("run"),
            )
            .unwrap();
        let pid = first.status().pid.unwrap();
        let fingerprint = process_start_fingerprint(pid).unwrap().unwrap();
        assert_eq!(
            first.status().connection_mode,
            Some(AppServerConnectionMode::Owned)
        );
        Self {
            root,
            profile,
            supervisors: vec![first],
            process: (pid, fingerprint),
        }
    }

    fn first(&self) -> AppServerSupervisor {
        self.supervisors[0].clone()
    }

    fn attach(&mut self) -> AppServerSupervisor {
        let client = AppServerSupervisor::new();
        client
            .start(
                &app_server_test_binary(),
                "test",
                &self.profile,
                &self.profile.join("run"),
            )
            .unwrap();
        assert_eq!(
            client.status().connection_mode,
            Some(AppServerConnectionMode::Attached)
        );
        assert_eq!(client.status().pid, Some(self.process.0));
        self.supervisors.push(client.clone());
        client
    }

    fn wait_for_exit(&self) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while process_start_fingerprint(self.process.0)
            .unwrap()
            .as_deref()
            == Some(self.process.1.as_str())
        {
            assert!(Instant::now() < deadline, "shared service did not exit");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        for supervisor in self.supervisors.iter().rev() {
            if let Err(error) = supervisor.shutdown() {
                eprintln!("shared_fixture_shutdown_unconfirmed:{error}");
            }
        }
        if std::thread::panicking() {
            eprintln!("shared_fixture_failure_evidence={}", self.root.display());
        } else {
            self.wait_for_exit();
            std::fs::remove_dir_all(&self.root).unwrap();
        }
    }
}

#[test]
fn owning_host_detaches_without_stopping_other_clients_and_the_last_client_retires_the_service() {
    let _guard = app_server_test_guard();
    let mut fixture = Fixture::new();
    let first = fixture.first();
    let second = fixture.attach();
    assert_eq!(
        first.shutdown_exclusive().unwrap_err(),
        "app_server_other_clients_active"
    );
    assert_eq!(
        first.restart().unwrap_err(),
        "app_server_other_clients_active"
    );
    assert_eq!(
        second.shutdown_exclusive().unwrap_err(),
        "app_server_lifecycle_ownership_required"
    );
    assert_eq!(first.status().phase, AppServerPhase::Running);
    first.shutdown().unwrap();
    assert_eq!(
        first.status().server_lifecycle_mode.as_deref(),
        Some("shared_continued")
    );
    assert_eq!(
        second.request_result("ping", &json!({})).unwrap()["protocolVersion"],
        PROTOCOL_VERSION
    );
    first
        .start(
            &app_server_test_binary(),
            "test",
            &fixture.profile,
            &fixture.profile.join("run"),
        )
        .unwrap();
    assert_eq!(
        first.status().connection_mode,
        Some(AppServerConnectionMode::Attached)
    );
    second.shutdown().unwrap();
    assert_eq!(
        first.request_result("ping", &json!({})).unwrap()["protocolVersion"],
        PROTOCOL_VERSION
    );
    first.shutdown().unwrap();
    fixture.wait_for_exit();
    assert_eq!(first.status().phase, AppServerPhase::Stopped);
}

#[test]
fn a_client_arriving_after_shutdown_preparation_prevents_execution_and_keeps_the_server_available()
{
    let _guard = app_server_test_guard();
    let mut fixture = Fixture::new();
    let first = fixture.first();
    let prepared = first
        .request_result("shutdown/prepare", &json!({}))
        .unwrap();
    assert_eq!(prepared["ready"], true);
    let second = fixture.attach();
    let executed = first
        .request_result("shutdown/execute", &json!({}))
        .unwrap();
    assert_eq!(executed["accepted"], false);
    assert!(first.continuation_confirmed(&executed));
    first.detach_continued_service().unwrap();
    assert!(second.request_result("ping", &json!({})).is_ok());
    second.shutdown().unwrap();
    fixture.wait_for_exit();
}

#[test]
fn an_unfinished_writer_blocks_owned_shutdown_without_releasing_the_occupation() {
    let _guard = app_server_test_guard();
    let fixture = Fixture::new();
    let first = fixture.first();
    let owner = crate::model_connections::execution_owner().unwrap();
    let writer=first.request_result("profileWriter/prepare",&json!({"clientOperationId":"shared-exit-unknown-writer","kind":"user_mcp","resourceId":"shared-service-test","resourceRevision":"v1","owner":owner,"launchProtocol":1,"processMode":"remote_request"})).unwrap();
    assert!(first.shutdown().is_err());
    assert_eq!(first.status().phase, AppServerPhase::Running);
    assert!(first.request_result("ping", &json!({})).is_ok());
    let listed = first
        .request_result("profileWriter/list", &json!({}))
        .unwrap();
    assert!(
        listed
            .as_array()
            .unwrap()
            .iter()
            .any(|lease| lease["leaseId"] == writer["leaseId"] && lease["state"] != "released")
    );
    first
        .request_result(
            "profileWriter/release",
            &json!({"leaseId":writer["leaseId"],"owner":owner,"localTransportFinished":true,"remoteOutcomeUnconfirmed":true}),
        )
        .unwrap();
    first.shutdown().unwrap();
    fixture.wait_for_exit();
}

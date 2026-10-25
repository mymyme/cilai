use super::AppServerSupervisor;
use contexthub_protocol::{
    LocalModelRuntime, ModelAuthMode, ModelConnection, ModelConnectionChange,
    ModelConnectionCommitParams, ModelConnectionConfig, ModelConnectionKind,
    ModelConnectionSavePrepareParams, ModelCredentialAction, ModelDescriptorSaveParams,
    ModelHistoryConsent, ModelNetworkConfig, ModelPurpose, ModelPurposeBinding,
    ModelPurposeSaveParams, ModelRuntimeKind, ModelSelection, ModelSelectionPolicy,
    ModelValidationLayer, ModelValidationOutcome, ModelValidationPrepareParams,
    ModelValidationRecord, ModelValidationReportParams,
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::json;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

mod build_location {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../scripts/personal-build-fixture.rs"
    ));
}

pub(crate) use build_location::personal_build_root;

pub(crate) fn fixture_root(label: &str) -> PathBuf {
    let root = personal_build_root()
        .join("target-runtime-audit/fixtures")
        .join(format!("{label}-{}", uuid::Uuid::new_v4().simple()));
    fs::create_dir_all(&root).unwrap();
    root
}

#[test]
fn fixture_build_root_is_shared_by_source_and_personal_staging_layouts() {
    let home = std::env::temp_dir().join("cilai-fixture-home");
    let project = std::env::temp_dir().join("cilai-fixture-layout");
    let source = project.join("06代码索引/cilai-workbench");
    let expected = home.join(".cilai-personal-builds/desktop");
    let registration = serde_json::json!({"schemaVersion": 1, "applicationId": "cn.cilai.personal.workbench", "buildRoot": expected, "sourceRoot": source, "projectRoot": project});
    for manifest in [
        source.join("crates/workbench-runtime"),
        expected.join("crates/workbench-runtime"),
        expected.join("runtime-source/crates/workbench-runtime"),
    ] {
        assert_eq!(
            build_location::personal_build_root_for(&manifest, &home, &registration).unwrap(),
            expected
        );
    }
    assert!(
        build_location::personal_build_root_for(
            &project.join("other/crates/workbench-runtime"),
            &home,
            &registration
        )
        .is_err()
    );
    assert!(
        build_location::personal_build_root_for(
            Path::new("relative/crates/workbench-runtime"),
            &home,
            &registration
        )
        .is_err()
    );
}

pub(crate) fn profile_data_root(owner: &Path) -> PathBuf {
    let profile_id = uuid::Uuid::new_v4().simple().to_string();
    let business = owner.join("cn.cilai.personal.workbench/business");
    let profile = business.join("profiles").join(&profile_id);
    contexthub_platform_adapter::security::ensure_restricted_directory(&profile).unwrap();
    let mut manifest = contexthub_platform_adapter::security::create_new_restricted_file(
        &business.join("local-profile.json"),
    )
    .unwrap();
    write!(
        manifest,
        "{}",
        json!({"schemaVersion":1,"profileId":profile_id,"displayName":"隔离状态机夹具"})
    )
    .unwrap();
    manifest.sync_all().unwrap();
    profile
}

fn call<P: Serialize, R: DeserializeOwned>(
    server: &AppServerSupervisor,
    method: &str,
    params: P,
) -> R {
    serde_json::from_value(
        server
            .request_result(method, &serde_json::to_value(params).unwrap())
            .unwrap(),
    )
    .unwrap()
}

pub(super) fn seed_simulated_connection(
    server: &AppServerSupervisor,
    model: &str,
) -> ModelSelection {
    let operation_id = uuid::Uuid::new_v4().simple().to_string();
    let prepared: ModelConnectionChange = call(
        server,
        "modelConnection/savePrepare",
        ModelConnectionSavePrepareParams {
            operation_id: operation_id.clone(),
            connection_id: None,
            expected_config_revision: None,
            config: ModelConnectionConfig {
                name: "隔离状态机夹具".into(),
                kind: ModelConnectionKind::Local,
                base_url: Some("http://127.0.0.1:9/v1".into()),
                local_runtime: Some(LocalModelRuntime::Ollama),
                auth_mode: ModelAuthMode::None,
                enabled: true,
                network: ModelNetworkConfig::default(),
            },
            credential_action: ModelCredentialAction::Clear,
            network_credential_action: ModelCredentialAction::Keep,
        },
    );
    let connection: ModelConnection = call(
        server,
        "modelConnection/saveCommit",
        ModelConnectionCommitParams {
            operation_id,
            credential_ref: None,
            network_credential_ref: None,
        },
    );
    assert_eq!(connection.connection_id, prepared.connection.connection_id);
    let _: serde_json::Value = call(
        server,
        "modelDescriptor/save",
        ModelDescriptorSaveParams {
            connection_id: connection.connection_id.clone(),
            model_id: model.into(),
            display_name: model.into(),
            expected_revision: None,
            context_window_tokens: None,
            input_modalities: vec!["text".into()],
            default_reasoning_effort: Some("high".into()),
            supported_reasoning_efforts: vec!["high".into()],
        },
    );
    let validation: ModelValidationRecord = call(
        server,
        "modelValidation/prepare",
        ModelValidationPrepareParams {
            runtime_kind: ModelRuntimeKind::Workbench,
            connection_id: connection.connection_id.clone(),
            model_id: Some(model.into()),
            layer: ModelValidationLayer::Tools,
            expected_config_revision: connection.config_revision,
            expected_auth_generation: connection.auth_generation,
        },
    );
    let simulated: ModelValidationRecord = call(
        server,
        "modelValidation/report",
        ModelValidationReportParams {
            validation_id: validation.validation_id,
            outcome: ModelValidationOutcome::Passed,
            reason: Some("isolated_fixture".into()),
            runtime_version: None,
            service_version: Some("isolated_fixture".into()),
        },
    );
    assert_eq!(simulated.reason.as_deref(), Some("isolated_fixture"));
    ModelSelection {
        connection_id: connection.connection_id,
        model_id: model.into(),
        reasoning_effort: None,
    }
}

pub(super) fn bind_purpose(
    server: &AppServerSupervisor,
    selection: &ModelSelection,
    purpose: ModelPurpose,
) {
    let binding_id = format!(
        "isolated-fixture-{}",
        serde_json::to_value(&purpose).unwrap().as_str().unwrap()
    );
    let _: ModelPurposeBinding = call(
        server,
        "modelPurpose/save",
        ModelPurposeSaveParams {
            binding: ModelPurposeBinding {
                binding_id,
                purpose,
                scope_id: None,
                policy: ModelSelectionPolicy::Explicit,
                selection: Some(selection.clone()),
                enabled: true,
                revision: 0,
                consent: Some(ModelHistoryConsent {
                    service_name: "隔离状态机夹具".into(),
                    history_scope: "仅隔离状态机测试资料".into(),
                    may_incur_cost: false,
                    confirmed_at: 1,
                }),
            },
            expected_revision: None,
        },
    );
}

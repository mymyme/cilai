use super::*;

fn manifest() -> serde_json::Value {
    json!({"schemaVersion":1,"contextTypes":[{"contextType":"note.current","schemaVersion":"1.0","dataSchema":{"type":"object","properties":{"text":{"type":"string","maxLength":2000}},"required":["text"],"additionalProperties":false}}],
        "uiActions":[],"model":true,"conversationHandoff":true,"readTools":[],"writeTools":[]})
}
fn permissions() -> Vec<String> {
    [
        "page_ai.context",
        "page_ai.model",
        "page_ai.conversation_handoff",
    ]
    .map(str::to_owned)
    .to_vec()
}

#[test]
fn an_existing_file_without_explicit_page_grants_does_not_enable_page_ai() {
    assert_eq!(
        declaration(
            "web_application",
            &[],
            Some(b"ordinary legacy JSON content")
        )
        .unwrap(),
        None
    );
    assert_eq!(
        declaration("web_application", &permissions(), None)
            .unwrap_err()
            .reason,
        "personal_page_manifest_missing"
    );
    let bytes = serde_json::to_vec(&manifest()).unwrap();
    assert!(
        declaration("web_application", &permissions(), Some(&bytes))
            .unwrap()
            .is_some()
    );
    assert!(declaration("web_application", &["page_ai.model".into()], Some(&bytes)).is_err());
    assert!(validate_permissions("local_mcp", &permissions()).is_err());
    let mut expanded = permissions();
    expanded.push("personal_file.write_confirmed".into());
    assert!(validate_permissions("web_application", &expanded).is_err());
}

#[test]
fn unknown_schema_contracts_and_unconfirmed_ui_actions_are_rejected() {
    for schema in [
        json!({"type":"object","$ref":"https://example.invalid/schema"}),
        json!({"type":"object","properties":{"n":{"type":"number","multipleOf":2}}}),
        json!({"type":"object","properties":{"s":{"type":"string","pattern":"["}}}),
    ] {
        assert!(schema::validate(&schema).is_err());
    }
    assert!(schema::validate(&json!({"type":"object","properties":{"code":{"type":"string","pattern":"^[a-z]+$"}},"additionalProperties":false})).is_ok());
    let mut input = manifest();
    input["uiActions"] = json!([{"actionId":"note.update","title":"保存","description":"保存当前内容","inputSchema":{"type":"object","additionalProperties":false},"riskLevel":"high","preconditions":[],"expectedEffects":["资料更新"],"confirmationPolicy":"automatic"}]);
    let mut grants = permissions();
    grants.push("page_ai.ui_actions_confirmed".into());
    assert!(
        declaration(
            "web_application",
            &grants,
            Some(&serde_json::to_vec(&input).unwrap())
        )
        .is_err()
    );
    input["uiActions"][0]["confirmationPolicy"] = json!("always");
    assert!(
        declaration(
            "web_application",
            &grants,
            Some(&serde_json::to_vec(&input).unwrap())
        )
        .is_ok()
    );
}

struct Fixture {
    base: PathBuf,
    profile: PathBuf,
    project: String,
    digest: String,
    services: AppServerServices,
}
impl Fixture {
    fn new() -> Self {
        Self::for_target("web_application")
    }
    fn for_target(target: &str) -> Self {
        let parent = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(4)
            .unwrap()
            .join(".personal-build/page-ai-fixtures");
        fs::create_dir_all(&parent).unwrap();
        reject_symlink(&parent).unwrap();
        let base = parent
            .canonicalize()
            .unwrap()
            .join(Uuid::new_v4().simple().to_string());
        let profile = base
            .join("cn.cilai.personal.workbench/business/profiles")
            .join(Uuid::new_v4().simple().to_string());
        fs::create_dir_all(&profile).unwrap();
        let db = super::super::data::test_support::ready_db(&profile, "contexthub.db", "page-test");
        let mut services = AppServerServices::with_data_dir(Some(db), Some(profile.clone()));
        let mut files = vec![
            json!({"path":"web/index.html","content":"<h1>笔记</h1>"}),
            json!({"path":SOURCE_PATH,"content":manifest().to_string()}),
        ];
        if target == "web_mcp_application" {
            files.push(json!({"path":"mcp/server.mjs","content":"export const page='local';"}));
        }
        let mut request: CapabilityCreationCreateParams = serde_json::from_value(json!({"title":"个人页面","businessGoal":"用户授权后分析本页","targetType":target,"conversationId":null,"requestedPermissions":permissions(),"acceptanceExamples":[{"input":"打开页面","expected":"只使用本页上下文"}],"sourceFiles":files,"clientOperationId":"create-page","requestFingerprint":""})).unwrap();
        request.request_fingerprint = capability_creation_request_fingerprint(&request);
        let created = services
            .capability_creation_create(&request, 1)
            .unwrap()
            .project;
        let digest = created.package_digest.unwrap();
        super::super::data::test_support::upgrade(&mut services, json!({"creationProjectId":created.creation_project_id,"expectedRevision":created.state_revision,"packageDigest":digest,"clientOperationId":"install-page","confirmedPermissions":permissions()}), "page-test", 3).unwrap();
        Self {
            base,
            profile,
            project: created.creation_project_id,
            digest,
            services,
        }
    }
    fn binding(&mut self) -> Result<PersonalPageBindingSnapshot, ServiceError> {
        self.services.personal_page_binding(
            &PersonalPageBindingParams {
                creation_project_id: self.project.clone(),
                expected_package_digest: self.digest.clone(),
            },
            5,
        )
    }
    fn disable(&mut self, operation: &str) {
        let project = self
            .services
            .persistence()
            .unwrap()
            .capability_creation_project(&self.project)
            .unwrap()
            .unwrap();
        self.services
            .personal_capability_manage(
                &PersonalCapabilityManageParams {
                    creation_project_id: self.project.clone(),
                    package_digest: self.digest.clone(),
                    expected_revision: project.state_revision,
                    action: "disable".into(),
                    client_operation_id: operation.into(),
                },
                13,
            )
            .unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.services.persistence.take();
        fs::remove_dir_all(&self.base).unwrap();
    }
}

#[test]
fn installed_page_binding_uses_real_profile_package_permissions_and_data_slot() {
    let mut f = Fixture::new();
    let first = f.binding().unwrap();
    assert_eq!(
        first.profile_id,
        f.profile.file_name().unwrap().to_str().unwrap()
    );
    assert_eq!(first.package_digest, f.digest);
    assert_eq!(
        first.application_id,
        format!(
            "personal.{}.app",
            f.project.trim_start_matches("capability_")
        )
    );
    assert_eq!(first.page_ai, serde_json::from_value(manifest()).unwrap());
    assert!(first.permission_digest.starts_with("sha256:"));
    assert!(first.data_revision.starts_with("sha256:"));
    assert!(Uuid::parse_str(&first.active_data_slot_id).is_ok());
    assert_eq!(first.runtime_binding, None);
    assert_eq!(f.binding().unwrap(), first);
    f.disable("disable-page");
    assert_eq!(f.binding().unwrap_err().reason, "personal_page_not_ready");
}

#[test]
fn combination_page_runtime_binding_uses_only_the_current_profile_and_active_data_slot() {
    let mut f = Fixture::for_target("web_mcp_application");
    let binding = f.binding().unwrap();
    let runtime = binding.runtime_binding.unwrap();
    let slot = f
        .services
        .persistence()
        .unwrap()
        .personal_application_data_slot(&binding.profile_id, &f.project)
        .unwrap()
        .unwrap();
    assert_eq!(
        runtime.data_root,
        f.profile.join(slot.relative_path).to_str().unwrap()
    );
    assert_eq!(
        runtime.package_root,
        f.profile
            .join("personal-capabilities")
            .join(&f.project)
            .join(f.digest.trim_start_matches("sha256:"))
            .to_str()
            .unwrap()
    );
    assert_eq!(
        runtime.harness_home,
        f.profile
            .join("ai-runtime/cilai-application-harness")
            .to_str()
            .unwrap()
    );
    assert_eq!(runtime.package_digest, f.digest);
}

#[test]
fn changed_installed_declaration_or_package_digest_cannot_reuse_a_page_binding() {
    let mut f = Fixture::new();
    let before = f.binding().unwrap();
    let root = f
        .profile
        .join("personal-capabilities")
        .join(&f.project)
        .join(f.digest.trim_start_matches("sha256:"));
    fs::write(root.join("source").join(SOURCE_PATH), b"{}").unwrap();
    assert!(f.binding().is_err());
    assert!(before.page_ai.model);
    let request = PersonalPageBindingParams {
        creation_project_id: f.project.clone(),
        expected_package_digest: format!("sha256:{}", "f".repeat(64)),
    };
    assert_eq!(
        f.services
            .personal_page_binding(&request, 6)
            .unwrap_err()
            .reason,
        "personal_page_binding_changed"
    );
}

fn audit_request(f: &mut Fixture) -> contexthub_protocol::PersonalPageAuditPrepareParams {
    let binding = f.binding().unwrap();
    contexthub_protocol::PersonalPageAuditPrepareParams {
        operation_id: Uuid::new_v4().simple().to_string(),
        binding: PersonalPageBindingParams {
            creation_project_id: f.project.clone(),
            expected_package_digest: f.digest.clone(),
        },
        permission_digest: binding.permission_digest,
        document_id: "page-document-1".into(),
        session_id: Some("page-session-1".into()),
        run_id: None,
        request_id: "request-1".into(),
        method: contexthub_protocol::PersonalPageAuditMethod::ContextPublish,
        arguments_digest: format!("sha256:{:x}", Sha256::digest("只保存摘要而非正文")),
    }
}

#[test]
fn page_audit_is_durable_exact_and_unknown_receipts_cannot_be_rewritten() {
    use contexthub_protocol::{
        PersonalPageAuditFinishParams, PersonalPageAuditListParams, PersonalPageAuditOutcome,
    };
    let mut f = Fixture::new();
    let p = audit_request(&mut f);
    let prepared = f.services.personal_page_audit_prepare(&p, 10).unwrap();
    assert_eq!(prepared.outcome, None);
    assert_eq!(
        f.services.personal_page_audit_prepare(&p, 11).unwrap(),
        prepared
    );
    let mut changed = p.clone();
    changed.request_id = "another-request".into();
    assert_eq!(
        f.services
            .personal_page_audit_prepare(&changed, 12)
            .unwrap_err()
            .reason,
        "personal_page_audit_operation_reused"
    );
    f.disable("disable-audited-page");
    let finish = PersonalPageAuditFinishParams {
        operation_id: p.operation_id.clone(),
        outcome: PersonalPageAuditOutcome::OutcomeUnknown,
        reason_code: Some("personal_page_outcome_unconfirmed".into()),
    };
    let receipt = f.services.personal_page_audit_finish(&finish, 14).unwrap();
    assert_eq!(
        f.services.personal_page_audit_finish(&finish, 15).unwrap(),
        receipt
    );
    let mut rewritten = finish.clone();
    rewritten.outcome = PersonalPageAuditOutcome::Succeeded;
    assert_eq!(
        f.services
            .personal_page_audit_finish(&rewritten, 16)
            .unwrap_err()
            .reason,
        "personal_page_audit_already_finished"
    );
    f.services.persistence.take();
    let db = contexthub_persistence_sqlite::SqlitePersistence::open(
        &f.profile.join("contexthub.db"),
        "test",
    )
    .unwrap();
    f.services = AppServerServices::with_data_dir(Some(db), Some(f.profile.clone()));
    let listing = f
        .services
        .personal_page_audit_list(
            &PersonalPageAuditListParams {
                creation_project_id: f.project.clone(),
                limit: 20,
                before_sequence: None,
            },
            17,
        )
        .unwrap();
    assert_eq!(listing.items.len(), 2);
    assert_eq!(listing.items[0].record, receipt);
    assert_eq!(listing.items[1].record, prepared);
    assert!(
        !serde_json::to_string(&listing)
            .unwrap()
            .contains("只保存摘要而非正文")
    );
    let foreign = "0".repeat(32);
    assert_eq!(
        f.services
            .persistence_mut()
            .unwrap()
            .finish_personal_page_audit(&foreign, &finish, 18)
            .unwrap_err()
            .reason(),
        Some("personal_page_audit_restored_read_only")
    );
    let historical = f
        .services
        .persistence()
        .unwrap()
        .personal_page_audits(
            &foreign,
            &PersonalPageAuditListParams {
                creation_project_id: f.project.clone(),
                limit: 1,
                before_sequence: None,
            },
        )
        .unwrap();
    assert!(historical.items[0].record.read_only_after_restore);
    assert!(historical.next_before_sequence.is_some());
}

#[test]
fn page_audit_rejects_ungranted_purposes_and_does_not_invent_missing_start_receipts() {
    use contexthub_protocol::{
        PersonalPageAuditFinishParams, PersonalPageAuditMethod, PersonalPageAuditOutcome,
    };
    let mut f = Fixture::new();
    let mut p = audit_request(&mut f);
    p.method = PersonalPageAuditMethod::ToolWrite;
    assert_eq!(
        f.services
            .personal_page_audit_prepare(&p, 10)
            .unwrap_err()
            .reason,
        "personal_page_ai_not_authorized"
    );
    let finish = PersonalPageAuditFinishParams {
        operation_id: p.operation_id.clone(),
        outcome: PersonalPageAuditOutcome::Succeeded,
        reason_code: None,
    };
    assert_eq!(
        f.services
            .personal_page_audit_finish(&finish, 11)
            .unwrap_err()
            .reason,
        "personal_page_audit_not_prepared"
    );
    p.method = PersonalPageAuditMethod::ContextPublish;
    p.document_id = "https://untrusted.example/document".into();
    assert_eq!(
        f.services
            .personal_page_audit_prepare(&p, 12)
            .unwrap_err()
            .reason,
        "personal_page_audit_request_invalid"
    );
}

#[test]
fn maintenance_refuses_new_page_work_but_preserves_the_real_existing_audit_result() {
    use contexthub_protocol::{
        PersonalApplicationDataAction, PersonalApplicationDataPrepareParams,
        PersonalPageAuditFinishParams, PersonalPageAuditOutcome,
    };
    let mut f = Fixture::new();
    let p = audit_request(&mut f);
    f.services.personal_page_audit_prepare(&p, 10).unwrap();
    let project = f
        .services
        .persistence()
        .unwrap()
        .capability_creation_project(&f.project)
        .unwrap()
        .unwrap();
    let execute = super::super::data::test_support::guarded(
        &mut f.services,
        &PersonalApplicationDataPrepareParams {
            creation_project_id: f.project.clone(),
            expected_revision: project.state_revision,
            package_digest: f.digest.clone(),
            client_operation_id: "maintenance-page".into(),
            action: PersonalApplicationDataAction::Upgrade,
            target_operation_id: None,
        },
        "page-test",
    );
    let mut another = p.clone();
    another.operation_id = Uuid::new_v4().simple().to_string();
    assert_eq!(
        f.services
            .personal_page_audit_prepare(&another, 11)
            .unwrap_err()
            .reason,
        "personal_data_maintenance_active"
    );
    let receipt = f
        .services
        .personal_page_audit_finish(
            &PersonalPageAuditFinishParams {
                operation_id: p.operation_id,
                outcome: PersonalPageAuditOutcome::Cancelled,
                reason_code: None,
            },
            12,
        )
        .unwrap();
    assert_eq!(receipt.outcome, Some(PersonalPageAuditOutcome::Cancelled));
    let proof = execute.quiescence.as_ref().unwrap();
    let status = f
        .services
        .persistence_mut()
        .unwrap()
        .personal_writer_rpc(
            "personalMaintenance/drainStatus",
            json!({"fenceId":proof.token,"owner":proof.owner}),
            &proof.profile_id,
            "page-test",
            13,
        )
        .unwrap();
    assert_eq!(status["fence"]["state"], "draining");
    f.services
        .personal_application_data_rpc(
            "personalApplicationData/cancel",
            json!({"operationId":execute.operation_id,"previewDigest":execute.preview_digest}),
            14,
        )
        .unwrap();
    super::super::data::test_support::release(&mut f.services, &execute, "page-test");
}

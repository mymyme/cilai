use super::{AiRuntimeManager, ApplicationAiContextSnapshot};
use crate::{application_tool_adapter::ScopeValidator, personal_page_ai::PageCallbacksFactory};
use serde_json::{Value, json};
use std::sync::Arc;

fn context() -> Value {
    json!({"applicationId":"personal.fixture","applicationName":"个人页面",
        "applicationIds":["personal.fixture"],"restrictToApplications":true,
        "version":"1","pageRevision":1,"contexts":[],"actions":[],"toolNames":[],"toolSetRevision":null})
}

fn never_factory() -> PageCallbacksFactory {
    Arc::new(|_, _| panic!("rejected execution must not acquire personal tools"))
}

#[test]
fn personal_page_execution_rejects_legacy_capabilities_before_any_model_or_scope_work() {
    let manager = AiRuntimeManager::new();
    let scope: ScopeValidator = Arc::new(|| panic!("legacy context must be rejected first"));
    let factory = never_factory();
    for (field, value) in [
        ("applicationIds", json!(["another.application"])),
        ("restrictToApplications", json!(false)),
        (
            "skillBinding",
            json!({"name":"old","contentDigest":"old","required":true}),
        ),
        (
            "modelTask",
            json!({"purpose":"old","taskId":"old","inputDigest":"old"}),
        ),
        ("browserOrigins", json!(["https://example.invalid"])),
        ("reportReview", json!(true)),
        ("localMaterialPaths", json!(["/not-authorized"])),
        ("toolNames", json!(["old-tool"])),
        (
            "readTools",
            json!([{"applicationId":"old","toolName":"read"}]),
        ),
        (
            "writeTools",
            json!([{"applicationId":"old","applicationName":"Old","dynamicName":"old",
            "toolName":"write","title":"Write","description":"old","inputSchema":{},"riskLevel":"high","timeoutMs":null}]),
        ),
        (
            "searchableApplications",
            json!([{"applicationId":"old","name":"Old","description":"old","category":"old","tools":[]}]),
        ),
        ("toolSetRevision", json!("old-revision")),
    ] {
        let mut input = context();
        input[field] = value;
        let input: ApplicationAiContextSnapshot = serde_json::from_value(input).unwrap();
        assert_eq!(
            manager
                .start_page_model_run_impl(
                    input,
                    "unused",
                    "context",
                    "user input",
                    "session",
                    "message",
                    &scope,
                    &factory
                )
                .unwrap_err(),
            "personal_page_legacy_context_not_supported",
            "{field}"
        );
    }
}

#[test]
fn personal_page_execution_checks_its_required_scope_before_resolving_a_connection() {
    let manager = AiRuntimeManager::new();
    let scope: ScopeValidator = Arc::new(|| Err("personal_page_scope_revoked".into()));
    let input = serde_json::from_value(context()).unwrap();
    assert_eq!(
        manager
            .start_page_model_run_impl(
                input,
                "unused",
                "context",
                "user input",
                "session",
                "message",
                &scope,
                &never_factory()
            )
            .unwrap_err(),
        "personal_page_scope_revoked"
    );
}

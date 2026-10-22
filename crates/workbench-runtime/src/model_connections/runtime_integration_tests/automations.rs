use super::*;
use crate::ai_runtime::{AutomationRuleRunRequest, AutomationRuleSaveRequest};
use contexthub_platform_adapter::power_epoch::{PowerEpoch, PowerEpochSnapshot, WakeBoundary};

pub(super) fn prepare_rule(fixture: &Fixture) -> crate::ai_runtime::AutomationRule {
    let root = fixture.owner.join("automation-workspace");
    fs::create_dir_all(&root).unwrap();
    let workspace = fixture.server.request_result("workspace/create", &json!({
        "localRoot":root, "displayName":"自动化隔离工作区", "clientOperationId":"sleep-workspace"
    })).unwrap();
    let project = fixture
        .server
        .request_result(
            "project/create",
            &json!({
                "workspaceId":workspace["workspace"]["workspaceId"], "workspaceIds":[],
                "displayName":"自动化睡眠验收", "goal":"仅本机受控协议测试",
                "clientOperationId":"sleep-project", "requestFingerprint":"sleep-project"
            }),
        )
        .unwrap();
    fixture.server.request_result("modelPurpose/save", &json!({
        "binding":{"bindingId":"sleep-automation-purpose", "purpose":"automation", "scopeId":null,
            "policy":"explicit", "selection":{"connectionId":fixture.connection.connection_id,
            "modelId":"isolated-official-fixture", "reasoningEffort":null}, "enabled":true,"revision":0,
            "consent":{"serviceName":"本机Responses仿真服务", "historyScope":"仅隔离验收资料",
            "mayIncurCost":false,"confirmedAt":1}},"expectedRevision":null
    })).unwrap();
    fixture.manager.model_catalog_status().unwrap();
    let request: AutomationRuleSaveRequest = serde_json::from_value(json!({
        "name":"原生睡眠边界", "intervalMinutes":15, "scheduleKind":"interval",
        "conditionSummary":"明确用户受理", "actionPrompt":"AUTOMATION_SLEEP_FIXTURE_V1 只回复只读结论",
        "workspaceId":workspace["workspace"]["workspaceId"], "workspaceName":"自动化隔离工作区",
        "workspaceRoot":root, "projectId":project["project"]["projectId"], "projectName":"自动化睡眠验收",
        "agentId":serde_json::to_value(fixture.manager.live_snapshot()).unwrap()["agents"][0]["agentId"],
        "modelId":"isolated-official-fixture", "enabled":true
    })).unwrap();
    fixture.manager.save_automation_rule(request).unwrap()
}

fn install_power_source(fixture: &Fixture) {
    let manager = fixture.manager.clone();
    let now = i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap();
    fixture.manager.set_automation_power_test_source(move || {
        let state = serde_json::to_value(manager.live_snapshot()).unwrap();
        let ready = state["runs"].as_array().unwrap().iter().any(|run| {
            run["events"]
                .as_array()
                .unwrap()
                .iter()
                .any(|event| event["eventType"] == "harness.thread_ready")
        });
        Ok(PowerEpochSnapshot {
            epoch: PowerEpoch {
                last_sleep: if ready { 3 } else { 1 },
                last_wake: if ready { 4 } else { 2 },
            },
            wake_boundary: Some(WakeBoundary {
                earliest_millis: now - 1,
                latest_millis: now - 1,
            }),
            clock_anchor: None,
        })
    });
}

fn await_sleep_claim_receipt(fixture: &Fixture, run_id: &str) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let page = fixture
            .manager
            .automation_trigger_history(
                &contexthub_protocol::PersonalAutomationListTriggersParams {
                    rule_id: None,
                    limit: 100,
                    before_sequence: None,
                },
            )
            .unwrap();
        if page.items.iter().any(|item| {
            item.record.run_id.as_deref() == Some(run_id)
                && item.record.outcome
                    == contexthub_protocol::PersonalAutomationTriggerOutcome::Cancelled
                && item.claim_released == Some(true)
                && item.record.reason_code.as_deref() == Some("automation_missed_during_sleep")
        }) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "sleep cancellation claim is not settled"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn official_automation_sleep_before_turn_start_never_sends_the_model_and_a_new_explicit_run_works()
{
    let _guard = crate::supervisor::app_server_test_guard();
    let fixture = Fixture::new();
    fixture.validate_tools();
    let baseline = fixture.count.load(Ordering::Acquire);
    let rule = prepare_rule(&fixture);
    install_power_source(&fixture);
    let run = fixture
        .manager
        .run_automation_rule(&AutomationRuleRunRequest {
            automation_rule_id: rule.automation_rule_id.clone(),
        })
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(40);
    loop {
        let state = serde_json::to_value(fixture.manager.live_snapshot()).unwrap();
        let current = state["runs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["runId"] == run.run_id)
            .unwrap();
        if current["status"] == "cancelled" && current["threadWriterActive"] == false {
            assert_eq!(current["errorReason"], "automation_missed_during_sleep");
            assert_eq!(current["automationRuleId"], rule.automation_rule_id);
            break;
        }
        assert!(
            Instant::now() < deadline,
            "{}",
            serde_json::to_string(current).unwrap()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(fixture.count.load(Ordering::Acquire), baseline);
    fixture.assert_released();
    await_sleep_claim_receipt(&fixture, &run.run_id);
    let resumed = fixture
        .manager
        .run_automation_rule(&AutomationRuleRunRequest {
            automation_rule_id: rule.automation_rule_id,
        })
        .unwrap();
    let completed = completed_run(&fixture.manager, &resumed.run_id);
    assert_eq!(completed["modelExecution"]["purpose"], "automation");
    assert!(fixture.count.load(Ordering::Acquire) > baseline);
    fixture.manager.shutdown_checked().unwrap();
    fixture.assert_released();
    let receipt: Value = serde_json::from_slice(
        &fs::read(
            fixture
                .profile
                .join("ai-runtime/runs")
                .join(format!("{}.thread-check.json", resumed.run_id)),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(receipt["status"], "matched", "{receipt}");
}

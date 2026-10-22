use super::*;
use crate::ai_runtime::AutomationRuleRunRequest;
use contexthub_protocol::{PersonalAutomationListTriggersParams, PersonalAutomationTriggerOutcome};

fn history(
    manager: &AiRuntimeManager,
    rule: &str,
) -> contexthub_protocol::PersonalAutomationTriggerPage {
    manager
        .automation_trigger_history(&PersonalAutomationListTriggersParams {
            rule_id: Some(rule.into()),
            limit: 100,
            before_sequence: None,
        })
        .unwrap()
}

fn wait_settled(manager: &AiRuntimeManager, rule: &str, run: &str) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if history(manager, rule).items.iter().any(|item| {
            item.record.run_id.as_deref() == Some(run)
                && item.record.outcome == PersonalAutomationTriggerOutcome::Completed
        }) {
            return;
        }
        assert!(Instant::now() < deadline, "automation claim remains active");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn run_other_host(fixture: &Fixture, rule: &str) -> Value {
    let output = fixture.owner.join("automation-peer-result.json");
    let log = fixture.owner.join("automation-peer.log");
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--ignored", "--exact", "model_connections::runtime_integration_tests::automation_overlap::automation_overlap_peer", "--nocapture"])
        .env("CILAI_AUTOMATION_PEER_PROFILE", &fixture.profile)
        .env("CILAI_AUTOMATION_PEER_RULE", rule)
        .env("CILAI_AUTOMATION_PEER_OUTPUT", &output)
        .stdin(std::process::Stdio::null())
        .stdout(fs::File::create(&log).unwrap())
        .stderr(fs::File::options().append(true).open(&log).unwrap())
        .spawn().unwrap();
    let deadline = Instant::now() + Duration::from_secs(40);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("automation peer did not exit: {}", log.display());
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(status.success(), "{}", fs::read_to_string(&log).unwrap());
    serde_json::from_slice(&fs::read(output).unwrap()).unwrap()
}

#[test]
fn official_automation_keeps_one_rule_claim_until_exit_and_other_hosts_record_skips() {
    let _guard = crate::supervisor::app_server_test_guard();
    let fixture = Fixture::new();
    fixture.validate_tools();
    let rule = super::automations::prepare_rule(&fixture);
    let request = AutomationRuleRunRequest {
        automation_rule_id: rule.automation_rule_id.clone(),
    };
    let baseline = fixture.count.load(Ordering::Acquire);
    fixture.hold.store(true, Ordering::Release);
    let run = fixture.manager.run_automation_rule(&request).unwrap();
    super::wait_for_requests(&fixture, baseline + 1);
    let first_count = serde_json::to_value(fixture.manager.live_snapshot()).unwrap()["runs"]
        .as_array()
        .unwrap()
        .len();
    assert_eq!(
        fixture.manager.run_automation_rule(&request).unwrap_err(),
        "automation_previous_run_active"
    );
    let peer = run_other_host(&fixture, &rule.automation_rule_id);
    assert_eq!(peer["reason"], "automation_previous_run_active");
    assert_eq!(peer["connectionMode"], "attached");
    assert_eq!(fixture.count.load(Ordering::Acquire), baseline + 1);
    let snapshot = serde_json::to_value(fixture.manager.live_snapshot()).unwrap();
    assert_eq!(snapshot["runs"].as_array().unwrap().len(), first_count);
    let triggers = history(&fixture.manager, &rule.automation_rule_id);
    assert_eq!(
        triggers
            .items
            .iter()
            .filter(|item| item.record.outcome == PersonalAutomationTriggerOutcome::Admitted)
            .count(),
        1
    );
    assert_eq!(
        triggers
            .items
            .iter()
            .filter(|item| item.record.outcome == PersonalAutomationTriggerOutcome::Skipped)
            .count(),
        2
    );
    assert!(
        triggers
            .items
            .iter()
            .filter(|item| item.record.outcome == PersonalAutomationTriggerOutcome::Skipped)
            .all(|item| item.record.reason_code.as_deref()
                == Some("automation_previous_run_active")
                && item.record.run_id.is_none())
    );
    fixture.hold.store(false, Ordering::Release);
    completed_run(&fixture.manager, &run.run_id);
    fixture.assert_released();
    wait_settled(&fixture.manager, &rule.automation_rule_id, &run.run_id);
    let next = fixture.manager.run_automation_rule(&request).unwrap();
    assert_ne!(next.run_id, run.run_id);
    completed_run(&fixture.manager, &next.run_id);
    wait_settled(&fixture.manager, &rule.automation_rule_id, &next.run_id);
    fixture.manager.shutdown_checked().unwrap();
    fixture.assert_released();
}

#[test]
#[ignore = "仅由父测试启动，验证真实第二宿主附着及同规则跳过"]
fn automation_overlap_peer() {
    let profile = PathBuf::from(std::env::var_os("CILAI_AUTOMATION_PEER_PROFILE").unwrap());
    let rule = std::env::var("CILAI_AUTOMATION_PEER_RULE").unwrap();
    let output = PathBuf::from(std::env::var_os("CILAI_AUTOMATION_PEER_OUTPUT").unwrap());
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
    let reason = manager
        .run_automation_rule(&AutomationRuleRunRequest {
            automation_rule_id: rule,
        })
        .unwrap_err();
    let result = json!({"reason":reason,"connectionMode":serde_json::to_value(server.status()).unwrap()["connectionMode"]});
    manager.shutdown_checked().unwrap();
    server.shutdown().unwrap();
    fs::write(output, serde_json::to_vec(&result).unwrap()).unwrap();
}

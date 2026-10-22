use super::*;

#[test]
fn official_guardian_usage_is_bound_to_its_own_connection_and_survives_restart() {
    let _guard = crate::supervisor::app_server_test_guard();
    let f = Fixture::new();
    f.validate_tools();
    let review_service = Fixture::new();
    let mut config = serde_json::to_value(&review_service.connection.config).unwrap();
    config["name"] = json!("本机独立审核用量");
    let prepared=f.server.request_result("modelConnection/savePrepare",&json!({"operationId":"usage-review-connection","config":config,"credentialAction":"clear"})).unwrap();
    let value = f
        .server
        .request_result(
            "modelConnection/saveCommit",
            &json!({"operationId":prepared["operationId"]}),
        )
        .unwrap();
    let reviewer: ModelConnection = serde_json::from_value(value).unwrap();
    assert_ne!(reviewer.connection_id, f.connection.connection_id);
    f.server.request_result("modelDescriptor/save",&json!({"connectionId":reviewer.connection_id,"modelId":"isolated-review-fixture","displayName":"独立审核用量模型"})).unwrap();
    f.validate_model(&reviewer, "isolated-review-fixture");
    f.server.request_result("modelPurpose/save",&json!({"binding":{"bindingId":"profile:review","purpose":"review","scopeId":null,"policy":"explicit","selection":{"connectionId":reviewer.connection_id,"modelId":"isolated-review-fixture","reasoningEffort":null},"enabled":true,"revision":0,"consent":{"serviceName":"本机独立审核服务","historyScope":"本轮审批历史","mayIncurCost":false,"confirmedAt":1}},"expectedRevision":null})).unwrap();
    let snapshot = serde_json::to_value(f.manager.live_snapshot()).unwrap();
    let started=f.manager.start_conversation_with_application_context(serde_json::from_value(json!({"title":"独立审核实际用量","conversationType":"task","mode":"edit","agentId":snapshot["agents"][0]["agentId"],"context":{"workspaceRoot":""},"content":"GUARDIAN_REVIEW_FIXTURE_V1","clientMessageId":"official-guardian-usage","modelSelection":{"connectionId":f.connection.connection_id,"modelId":"isolated-official-fixture","reasoningEffort":null},"modelOverride":"isolated-official-fixture","executionTarget":"model","permissionMode":"auto-review","planning":false})).unwrap(),None,None,None).unwrap();
    let run = completed_run(&f.manager, &started.run.run_id);
    assert_eq!(run["output"], "fixture completed");
    f.assert_released();
    let requests = f.count.load(Ordering::Acquire);
    let review_requests = review_service.count.load(Ordering::Acquire);
    let guardian_count = (0..review_requests)
        .filter(|index| {
            let metadata: Value = serde_json::from_slice(
                &fs::read(
                    review_service
                        .owner
                        .join(format!("fixture-metadata-{index}.json")),
                )
                .unwrap(),
            )
            .unwrap();
            metadata["subagent"] == "guardian"
        })
        .count();
    assert_eq!(guardian_count, 1);
    let usage = f.manager.usage_view().unwrap();
    let rows = usage["rows"].as_array().unwrap();
    let review = rows
        .iter()
        .filter(|row| row["purpose"] == "review")
        .collect::<Vec<_>>();
    assert_eq!(review.len(), 1);
    let row = review[0];
    assert_eq!(row["connectionId"], reviewer.connection_id);
    assert_eq!(row["modelId"], "isolated-review-fixture");
    assert_eq!(row["phase"], "request");
    assert_eq!(row["tokens"]["inputTokens"], 1);
    assert_eq!(row["tokens"]["outputTokens"], 1);
    assert!(row["tokens"]["cachedInputTokens"].is_null());
    assert!(row["cost"].is_null());
    assert_eq!(row["status"], "completed");
    assert_eq!(row["recordSource"], "persisted_request_metadata");
    assert_ne!(row["runId"], run["runId"]);
    assert_eq!(
        rows.iter()
            .filter(|row| row["recordSource"] == "persisted_request_metadata")
            .count(),
        1
    );
    assert!(
        !usage
            .to_string()
            .contains("isolated explicit reviewer denial")
    );
    assert_eq!(f.count.load(Ordering::Acquire), requests);
    assert_eq!(
        review_service.count.load(Ordering::Acquire),
        review_requests
    );
    let request_id = row["runId"].clone();
    f.manager.shutdown_checked().unwrap();
    let reopened = AiRuntimeManager::new();
    reopened.initialize(&f.profile, &f.server).unwrap();
    let stored = reopened.usage_view().unwrap();
    let stored = stored["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["runId"] == request_id)
        .unwrap();
    assert_eq!(stored["connectionId"], reviewer.connection_id);
    assert_eq!(stored["tokens"]["inputTokens"], 1);
    assert_eq!(f.count.load(Ordering::Acquire), requests);
    assert_eq!(
        review_service.count.load(Ordering::Acquire),
        review_requests
    );
    reopened.shutdown_checked().unwrap();
    review_service.manager.shutdown_checked().unwrap();
}

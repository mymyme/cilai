use super::*;

pub(super) fn respond_failure(socket: &mut TcpStream, status: u16) -> bool {
    if status == 200 {
        let response = json!({"id":"synthetic-empty-response","object":"response","status":"completed","output":[],"usage":{"input_tokens":1,"output_tokens":0,"total_tokens":1}});
        let body = format!(
            "data: {}\n\ndata: {}\n\n",
            json!({"type":"response.created","response":response}),
            json!({"type":"response.completed","response":response})
        );
        write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
        return true;
    }
    let reason = match status {
        0 => return false,
        400 => "Bad Request",
        401 => "Unauthorized",
        402 => "Payment Required",
        403 => "Forbidden",
        422 => "Unprocessable Entity",
        429 => "Too Many Requests",
        _ => panic!("unexpected fixture status"),
    };
    let mut error = json!({"code":"INSUFFICIENT_CREDITS","message":"Authorization: Bearer isolated-private-error-body 企业员工额度"});
    if matches!(status, 400 | 422) {
        error["code"] = json!("invalid_request_error");
        error["param"] = json!(if status == 400 {
            "tool_choice"
        } else {
            "parallel_tool_calls"
        });
    }
    let body = json!({"error":error,"headers":{"Authorization":"isolated-private-error-body"},"request_id":"isolated-private-error-body"}).to_string();
    write!(socket,"HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
    true
}

#[test]
fn official_rejections_keep_safe_status_and_parameter_without_retries() {
    let _guard = crate::supervisor::app_server_test_guard();
    for (status, parameter) in [(400, "tool_choice"), (422, "parallel_tool_calls")] {
        let fixture = Fixture::new();
        fixture.validate_tools();
        let before = fixture.count.load(Ordering::Acquire);
        fixture.failure_status.store(status, Ordering::Release);
        let started = fixture.start();
        let run = terminal_run(&fixture.manager, &started.run.run_id);
        assert_eq!(run["status"], "failed");
        assert_eq!(fixture.count.load(Ordering::Acquire), before + 1);
        assert_eq!(
            run["errorReason"],
            format!(
                "模型服务拒绝了本次请求（HTTP {status}，参数：{parameter}），请检查接口与参数兼容性。\n错误代码：MODEL_REQUEST_REJECTED"
            )
        );
        assert_safe_native_audit(&fixture, &started.run.run_id, status, parameter);
        fixture.assert_released();
        fixture.manager.shutdown_checked().unwrap();
        let restored = AiRuntimeManager::new();
        restored
            .initialize(&fixture.profile, &fixture.server)
            .unwrap();
        let history = serde_json::to_value(restored.live_snapshot()).unwrap();
        let persisted = history["runs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["runId"] == started.run.run_id)
            .unwrap();
        assert_eq!(persisted["errorReason"], run["errorReason"]);
        restored.shutdown_checked().unwrap();
        assert_eq!(fixture.count.load(Ordering::Acquire), before + 1);
    }
}

fn assert_safe_native_audit(fixture: &Fixture, run: &str, status: u16, parameter: &str) {
    let text = fs::read_to_string(
        fixture
            .profile
            .join("ai-runtime/runs")
            .join(format!("{run}.harness.jsonl")),
    )
    .unwrap();
    let events = text
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert!(
        events
            .iter()
            .any(|event| event["runtimeVersion"] == "0.163.0")
    );
    let errors = events
        .iter()
        .filter(|event| !event["serviceError"].is_null())
        .collect::<Vec<_>>();
    assert!(!errors.is_empty());
    for event in errors {
        assert_eq!(
            event["serviceError"],
            json!({"upstreamStatus":status,"code":"invalid_request_error","parameter":parameter,
                "classification":{"reason":"unknown","shape":"error_object","reasonWords":[],
                    "parameterHints":[{"parameter":parameter,"source":"explicit_param"}]}})
        );
    }
    for private in ["isolated-private-error-body", "Authorization", "request_id"] {
        assert!(!text.contains(private));
    }
}

#[test]
fn official_http_failures_have_personal_reasons_without_retries_or_provider_body() {
    let _guard = crate::supervisor::app_server_test_guard();
    for (status, expected, code) in [
        (
            401,
            "模型服务认证失败，请检查该连接的凭据。",
            "MODEL_AUTH_REJECTED",
        ),
        (
            402,
            "模型服务余额或额度不足，请查看该服务的用量与账单。",
            "MODEL_REQUEST_REJECTED",
        ),
        (
            403,
            "模型服务拒绝访问，请检查该连接的账号与模型权限。",
            "MODEL_AUTH_REJECTED",
        ),
        (
            429,
            "模型服务请求过于频繁，请稍后重新发起。",
            "MODEL_RATE_LIMITED",
        ),
    ] {
        let fixture = Fixture::new();
        fixture.validate_tools();
        let before = fixture.count.load(Ordering::Acquire);
        fixture.failure_status.store(status, Ordering::Release);
        let started = fixture.start();
        let run = terminal_run(&fixture.manager, &started.run.run_id);
        assert_eq!(run["status"], "failed");
        assert_eq!(fixture.count.load(Ordering::Acquire), before + 1);
        assert_eq!(run["errorReason"], format!("{expected}\n错误代码：{code}"));
        let public = run["errorReason"].as_str().unwrap();
        for private in [
            "isolated-private-error-body",
            "Authorization",
            "企业",
            "员工",
            "127.0.0.1",
        ] {
            assert!(!public.contains(private));
        }
        fixture.assert_released();
        fixture.manager.shutdown_checked().unwrap();
        let restored = AiRuntimeManager::new();
        restored
            .initialize(&fixture.profile, &fixture.server)
            .unwrap();
        let history = serde_json::to_value(restored.live_snapshot()).unwrap();
        let persisted = history["runs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["runId"] == started.run.run_id)
            .unwrap();
        assert_eq!(persisted["errorReason"], run["errorReason"]);
        restored.shutdown_checked().unwrap();
        assert_eq!(fixture.count.load(Ordering::Acquire), before + 1);
    }
}

#[test]
fn official_empty_completed_response_is_failed_precisely_without_retry() {
    let _guard = crate::supervisor::app_server_test_guard();
    let fixture = Fixture::new();
    fixture.validate_tools();
    fixture.failure_status.store(200, Ordering::Release);
    for layer in [
        contexthub_protocol::ModelValidationLayer::Protocol,
        contexthub_protocol::ModelValidationLayer::Streaming,
    ] {
        let before = fixture.count.load(Ordering::Acquire);
        let record = fixture
            .manager
            .personal_model_validate(contexthub_protocol::ModelValidationPrepareParams {
                runtime_kind: ModelRuntimeKind::Workbench,
                connection_id: fixture.connection.connection_id.clone(),
                model_id: Some("isolated-official-fixture".into()),
                layer,
                expected_config_revision: fixture.connection.config_revision,
                expected_auth_generation: fixture.connection.auth_generation,
            })
            .unwrap();
        assert_eq!(
            record.outcome,
            Some(contexthub_protocol::ModelValidationOutcome::Failed)
        );
        assert_eq!(
            record.reason.as_deref(),
            Some("model_validation_empty_response")
        );
        assert_eq!(fixture.count.load(Ordering::Acquire), before + 1);
        fixture.assert_released();
    }
}

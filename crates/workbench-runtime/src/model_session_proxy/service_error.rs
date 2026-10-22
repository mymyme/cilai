use reqwest::{Response, StatusCode};
use serde::Serialize;
use serde_json::{Value, json};
use std::time::Duration;

const BODY_LIMIT: usize = 16 * 1024;
const BODY_TIMEOUT: Duration = Duration::from_secs(1);
pub(crate) mod classification;

const MARKER: &str = "[CILAI_MODEL_SERVICE_ERROR_V3 ";
const CLASSIFIED_MARKER: &str = "[CILAI_MODEL_SERVICE_ERROR_V2 ";
const LEGACY_MARKER: &str = "[CILAI_MODEL_SERVICE_ERROR_V1 ";

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ServiceError {
    pub upstream_status: u16,
    pub code: &'static str,
    pub parameter: &'static str,
    pub classification: classification::Classification,
}

pub(crate) fn model_code(status: u16) -> &'static str {
    match status {
        401 | 403 => "MODEL_AUTH_REJECTED",
        407 => "MODEL_PROXY_AUTHENTICATION_REJECTED",
        429 => "MODEL_RATE_LIMITED",
        500..=599 => "MODEL_SERVICE_UNAVAILABLE",
        _ => "MODEL_REQUEST_REJECTED",
    }
}

fn allowed_code(value: &str) -> &'static str {
    match value {
        "invalid_api_key" => "invalid_api_key",
        "authentication_error" => "authentication_error",
        "rate_limit_exceeded" => "rate_limit_exceeded",
        "insufficient_quota" => "insufficient_quota",
        "model_not_found" => "model_not_found",
        "invalid_request_error" => "invalid_request_error",
        "context_length_exceeded" => "context_length_exceeded",
        "unsupported_parameter" => "unsupported_parameter",
        "unknown_parameter" => "unknown_parameter",
        "invalid_value" => "invalid_value",
        _ => "unknown",
    }
}

fn allowed_parameter(value: &str) -> &'static str {
    match value {
        "model" => "model",
        "input" => "input",
        "tools" => "tools",
        "tool_choice" => "tool_choice",
        "parallel_tool_calls" => "parallel_tool_calls",
        "stream" => "stream",
        "store" => "store",
        "include" => "include",
        "reasoning" => "reasoning",
        "reasoning.effort" => "reasoning.effort",
        "reasoning.summary" => "reasoning.summary",
        "text" => "text",
        "text.format" => "text.format",
        "text.verbosity" => "text.verbosity",
        "max_output_tokens" => "max_output_tokens",
        "previous_response_id" => "previous_response_id",
        "truncation" => "truncation",
        "temperature" => "temperature",
        "top_p" => "top_p",
        "service_tier" => "service_tier",
        _ => "unknown",
    }
}

impl ServiceError {
    fn unknown(status: StatusCode) -> Self {
        Self {
            upstream_status: status.as_u16(),
            code: "unknown",
            parameter: "unknown",
            classification: classification::unknown(),
        }
    }

    fn from_body(status: StatusCode, bytes: &[u8]) -> Self {
        let mut diagnostic = Self::unknown(status);
        if bytes.len() <= BODY_LIMIT
            && let Ok(value) = serde_json::from_slice::<Value>(bytes)
        {
            diagnostic.classification = classification::classify(&value);
            diagnostic.code = allowed_code(value["error"]["code"].as_str().unwrap_or_default());
            diagnostic.parameter =
                allowed_parameter(value["error"]["param"].as_str().unwrap_or_default());
        }
        diagnostic
    }

    pub(crate) fn message(&self) -> String {
        format!(
            "{MARKER}code={} upstream_status={} parameter={} service_code={} reason={} shape={} hints={} words={}]",
            model_code(self.upstream_status),
            self.upstream_status,
            self.parameter,
            self.code,
            self.classification.reason,
            self.classification.shape,
            self.classification.wire_hints(),
            self.classification.wire_words(),
        )
    }

    pub(crate) fn response(&self) -> Value {
        json!({"error":{
            "code":model_code(self.upstream_status),
            "message":self.message(),
            "serviceError":self
        }})
    }

    fn legacy_message(&self) -> String {
        format!(
            "{LEGACY_MARKER}code={} upstream_status={} parameter={} service_code={}]",
            model_code(self.upstream_status),
            self.upstream_status,
            self.parameter,
            self.code
        )
    }

    pub(crate) fn from_native_message(message: &str) -> Option<Self> {
        let (start, prefix) = message
            .find(MARKER)
            .map(|start| (start, MARKER))
            .or_else(|| {
                message
                    .find(CLASSIFIED_MARKER)
                    .map(|start| (start, CLASSIFIED_MARKER))
            })
            .or_else(|| {
                message
                    .find(LEGACY_MARKER)
                    .map(|start| (start, LEGACY_MARKER))
            })?;
        let end = message.get(start..)?.find(']')? + start + 1;
        let marker = message
            .get(start..end)
            .filter(|value| value.len() <= 2048)?;
        let fields = marker.strip_prefix(prefix)?.strip_suffix(']')?;
        let mut fields = fields.split_ascii_whitespace();
        let code = fields.next()?.strip_prefix("code=")?;
        let status: u16 = fields
            .next()?
            .strip_prefix("upstream_status=")?
            .parse()
            .ok()?;
        if !(300..=599).contains(&status) {
            return None;
        }
        let parameter = fields.next()?.strip_prefix("parameter=")?;
        let service_code = fields.next()?.strip_prefix("service_code=")?;
        let classification = if prefix == LEGACY_MARKER {
            classification::unknown()
        } else {
            let reason = fields.next()?.strip_prefix("reason=")?;
            let shape = fields.next()?.strip_prefix("shape=")?;
            let hints = fields.next()?.strip_prefix("hints=")?;
            let words = if prefix == MARKER {
                fields.next()?.strip_prefix("words=")?
            } else {
                "none"
            };
            classification::from_wire(reason, shape, hints, words)?
        };
        let diagnostic = Self {
            upstream_status: status,
            code: allowed_code(service_code),
            parameter: allowed_parameter(parameter),
            classification,
        };
        let canonical = if prefix == MARKER {
            diagnostic.message()
        } else if prefix == CLASSIFIED_MARKER {
            diagnostic
                .message()
                .replace(MARKER, CLASSIFIED_MARKER)
                .strip_suffix(" words=none]")?
                .to_owned()
                + "]"
        } else {
            diagnostic.legacy_message()
        };
        (fields.next().is_none() && model_code(status) == code && canonical == marker)
            .then_some(diagnostic)
    }
}

pub(super) async fn read(mut response: Response) -> ServiceError {
    let status = response.status();
    if response
        .content_length()
        .is_some_and(|size| size > BODY_LIMIT as u64)
    {
        return ServiceError::unknown(status);
    }
    let body = tokio::time::timeout(BODY_TIMEOUT, async {
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.ok()? {
            if bytes.len().checked_add(chunk.len())? > BODY_LIMIT {
                return None;
            }
            bytes.extend_from_slice(&chunk);
        }
        Some(bytes)
    })
    .await;
    match body {
        Ok(Some(bytes)) => ServiceError::from_body(status, &bytes),
        _ => ServiceError::unknown(status),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read as _, Write as _};

    #[test]
    fn rejection_metadata_is_an_exact_allowlist_without_free_provider_text() {
        for (status, parameter) in [(400, "tool_choice"), (422, "parallel_tool_calls")] {
            let diagnostic = ServiceError::from_body(
                StatusCode::from_u16(status).unwrap(),
                json!({"error":{"code":"invalid_request_error","param":parameter,
                    "message":"Authorization: Bearer private-fixture", "headers":{"secret":"private-fixture"}},
                    "request_id":"private-fixture"}).to_string().as_bytes(),
            );
            assert_eq!(diagnostic.upstream_status, status);
            assert_eq!(diagnostic.parameter, parameter);
            assert_eq!(diagnostic.code, "invalid_request_error");
            let public = diagnostic.response().to_string();
            for secret in ["private-fixture", "Authorization", "headers", "request_id"] {
                assert!(!public.contains(secret));
            }
            assert_eq!(ServiceError::from_native_message(&public), Some(diagnostic));
        }
    }

    #[test]
    fn unknown_invalid_or_oversized_metadata_never_becomes_a_marker() {
        let oversized = json!({"error":{"code":"invalid_request_error","param":"tools"},"padding":"x".repeat(BODY_LIMIT)}).to_string();
        for body in [
            br#"{"error":{"code":"private-fixture","param":"tools private-fixture","message":"private-fixture"}}"#.as_slice(),
            b"private-fixture",
            oversized.as_bytes(),
        ] {
            let diagnostic = ServiceError::from_body(StatusCode::BAD_REQUEST, body);
            assert_eq!(diagnostic.code, "unknown");
            assert_eq!(diagnostic.parameter, "unknown");
            assert!(!diagnostic.response().to_string().contains("private-fixture"));
        }
        for marker in [
            "[CILAI_MODEL_SERVICE_ERROR_V1 code=MODEL_AUTH_REJECTED upstream_status=400 parameter=tools service_code=unknown]",
            "[CILAI_MODEL_SERVICE_ERROR_V1 code=MODEL_REQUEST_REJECTED upstream_status=400 parameter=private-fixture service_code=unknown]",
            "[CILAI_MODEL_SERVICE_ERROR_V1 code=MODEL_REQUEST_REJECTED upstream_status=400 parameter=tools service_code=private-fixture]",
            "[CILAI_MODEL_SERVICE_ERROR_V1 code=MODEL_REQUEST_REJECTED upstream_status=200 parameter=tools service_code=unknown]",
        ] {
            assert_eq!(ServiceError::from_native_message(marker), None);
        }
    }

    #[test]
    fn classified_errors_survive_v3_markers_and_legacy_markers_remain_bounded() {
        let diagnostic = ServiceError::from_body(StatusCode::BAD_REQUEST, br#"{"error":{"type":"unsupported_parameter","param":"tool_choice","message":"private-fixture"}}"#);
        assert_eq!(diagnostic.classification.reason, "unsupported_parameter");
        assert_eq!(
            diagnostic.classification.wire_hints(),
            "tool_choice:explicit_param"
        );
        assert_eq!(
            ServiceError::from_native_message(&diagnostic.message()),
            Some(diagnostic.clone())
        );
        assert!(
            !diagnostic
                .response()
                .to_string()
                .contains("private-fixture")
        );
        let legacy = ServiceError::from_native_message("[CILAI_MODEL_SERVICE_ERROR_V1 code=MODEL_REQUEST_REJECTED upstream_status=400 parameter=tools service_code=unsupported_parameter]").unwrap();
        assert_eq!(legacy.parameter, "tools");
        assert_eq!(legacy.classification, classification::unknown());
        let v2 = "[CILAI_MODEL_SERVICE_ERROR_V2 code=MODEL_REQUEST_REJECTED upstream_status=400 parameter=tools service_code=unsupported_parameter reason=unsupported_parameter shape=error_object hints=tools:explicit_param]";
        assert!(
            ServiceError::from_native_message(v2)
                .unwrap()
                .classification
                .reason_words
                .is_empty()
        );
        let words = ServiceError::from_body(
            StatusCode::BAD_REQUEST,
            br#"{"error":{"message":"tools must not be empty private-fixture"}}"#,
        );
        assert_eq!(
            ServiceError::from_native_message(&words.message()),
            Some(words.clone())
        );
        assert!(!words.message().contains("private-fixture"));
        assert!(
            ServiceError::from_native_message(
                &words.message().replace("w:tools", "w:private-fixture")
            )
            .is_none()
        );
        for marker in [
            diagnostic
                .message()
                .replace("reason=unsupported_parameter", "reason=private-fixture"),
            diagnostic
                .message()
                .replace("tool_choice:explicit_param", "tool_choice:private-fixture"),
            diagnostic
                .message()
                .replace("shape=error_object", "shape=private-fixture"),
            diagnostic.message().replace(
                "hints=tool_choice:explicit_param",
                "hints=tool_choice:explicit_param,tool_choice:explicit_param",
            ),
        ] {
            assert!(ServiceError::from_native_message(&marker).is_none());
        }
    }

    fn rejection_response(
        case: &str,
    ) -> (
        u16,
        std::sync::Arc<std::sync::atomic::AtomicBool>,
        std::thread::JoinHandle<()>,
    ) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let case = case.to_owned();
        let sent = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let body_sent = sent.clone();
        let worker = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut request = [0; 4096];
            assert!(socket.read(&mut request).unwrap() > 0);
            match case.as_str() {
                "malformed" => {
                    let body = b"private-fixture";
                    write!(socket,"HTTP/1.1 422 Unprocessable Entity\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",body.len()).unwrap();
                    socket.write_all(body).unwrap();
                }
                "declared_oversize" => {
                    write!(socket,"HTTP/1.1 422 Unprocessable Entity\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",BODY_LIMIT+1).unwrap();
                }
                "chunk_oversize" => {
                    let body = json!({"error":{"code":"invalid_request_error","param":"tools"},"padding":"x".repeat(BODY_LIMIT)}).to_string();
                    write!(socket,"HTTP/1.1 422 Unprocessable Entity\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n",body.len()).unwrap();
                    let _ = socket.write_all(body.as_bytes());
                    let _ = socket.write_all(b"\r\n0\r\n\r\n");
                }
                "delayed" => {
                    socket.write_all(b"HTTP/1.1 422 Unprocessable Entity\r\nContent-Length: 2\r\nConnection: close\r\n\r\n").unwrap();
                    std::thread::sleep(BODY_TIMEOUT + Duration::from_millis(300));
                    body_sent.store(true, std::sync::atomic::Ordering::Release);
                    let _ = socket.write_all(b"{}");
                }
                _ => panic!("unexpected case"),
            }
        });
        (port, sent, worker)
    }

    #[test]
    fn bounded_rejection_reads_keep_status_when_body_cannot_be_classified() {
        for case in [
            "malformed",
            "declared_oversize",
            "chunk_oversize",
            "delayed",
        ] {
            let (port, sent, worker) = rejection_response(case);
            let diagnostic = crate::async_support::block_on(async {
                let response = reqwest::Client::builder()
                    .no_proxy()
                    .retry(reqwest::retry::never())
                    .build()
                    .unwrap()
                    .get(format!("http://127.0.0.1:{port}/"))
                    .send()
                    .await
                    .unwrap();
                read(response).await
            });
            assert_eq!(
                diagnostic,
                ServiceError::unknown(StatusCode::UNPROCESSABLE_ENTITY)
            );
            assert!(
                !diagnostic
                    .response()
                    .to_string()
                    .contains("private-fixture")
            );
            if case == "delayed" {
                assert!(!sent.load(std::sync::atomic::Ordering::Acquire));
            }
            worker.join().unwrap();
        }
    }
}

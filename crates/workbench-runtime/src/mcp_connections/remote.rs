use super::{Execution, Secret, digest};
use contexthub_protocol::{
    McpAuthMode, McpConnection, McpConnectionConfig, McpSource, McpToolDefinition,
    ModelNetworkConfig,
};
use serde_json::{Value, json};
use std::time::Duration;

const LIMIT: usize = 1024 * 1024;
pub(super) struct Evidence {
    pub protocol: String,
    pub name: String,
    pub version: String,
    pub fingerprint: String,
    pub tools: Vec<McpToolDefinition>,
}
pub(super) struct Session {
    client: reqwest::Client,
    url: reqwest::Url,
    secret: Option<Secret>,
    remote_id: Option<String>,
    protocol: Option<String>,
    sequence: u64,
    config: McpConnectionConfig,
}
impl Session {
    pub fn open(
        conn: &McpConnection,
        secret: Option<Secret>,
        vault_root: &std::path::Path,
    ) -> Result<Self, String> {
        let McpSource::RemoteHttp { url } = &conn.config.source else {
            return Err("mcp_local_isolation_not_supported".into());
        };
        let url = reqwest::Url::parse(url).map_err(|_| "mcp_remote_source_invalid")?;
        if !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || !(url.scheme() == "https"
                || url.scheme() == "http"
                    && matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]")))
        {
            return Err("mcp_remote_source_invalid".into());
        }
        if (conn.config.auth_mode == McpAuthMode::Bearer) != secret.is_some()
            || conn.config.auth_mode == McpAuthMode::Environment
        {
            return Err("mcp_auth_mode_invalid".into());
        }
        let network = ModelNetworkConfig {
            mode: conn.config.network.mode.clone(),
            proxy_url: conn.config.network.proxy_url.clone(),
            proxy_secret_ref: None,
            ca_certificate_pem: None,
            ..Default::default()
        };
        let client = crate::model_connections::connection_client(&network, vault_root)?;
        Ok(Self {
            client,
            url,
            secret,
            remote_id: None,
            protocol: None,
            sequence: 0,
            config: conn.config.clone(),
        })
    }
    fn safe(&self, value: &Value) -> Result<(), String> {
        if let Some(secret) = &self.secret {
            let encoded = serde_json::to_string(value).map_err(|_| "mcp_response_invalid")?;
            let escaped =
                serde_json::to_string(secret.expose()).map_err(|_| "mcp_credential_invalid")?;
            if encoded.contains(secret.expose())
                || escaped.len() > 2 && encoded.contains(&escaped[1..escaped.len() - 1])
            {
                return Err("mcp_response_contains_credentials".into());
            }
        }
        Ok(())
    }
    pub async fn request(
        &mut self,
        execution: &mut Execution,
        method: &str,
        params: Value,
        side_effect: bool,
    ) -> Result<Value, String> {
        execution.validate()?;
        self.sequence += 1;
        let id = self.sequence;
        let value = json!({"jsonrpc":"2.0","id":id,"method":method,"params":params});
        let response = self.post(execution, &value, side_effect).await?;
        let result = parse_response(&response.0, &response.1, id)?;
        self.safe(&result)?;
        execution.validate()?;
        Ok(result)
    }
    async fn post(
        &mut self,
        execution: &mut Execution,
        value: &Value,
        side_effect: bool,
    ) -> Result<(String, Vec<u8>), String> {
        execution.validate()?;
        let mut request = self
            .client
            .post(self.url.clone())
            .header("Accept", "application/json, text/event-stream")
            .timeout(Duration::from_secs(45))
            .json(value);
        if let Some(secret) = &self.secret {
            request = request.bearer_auth(secret.expose());
        }
        if let Some(session) = &self.remote_id {
            request = request.header("Mcp-Session-Id", session);
        }
        if let Some(protocol) = &self.protocol {
            request = request.header("MCP-Protocol-Version", protocol);
        }
        // Once dispatched, loss of the response does not establish that a remote tool did nothing.
        execution.request_sent |= side_effect;
        let response = request.send().await.map_err(|_| {
            if side_effect {
                "mcp_tool_outcome_unconfirmed"
            } else {
                "mcp_network_failed"
            }
        })?;
        if !response.status().is_success() {
            return Err(match response.status().as_u16() {
                401 => "mcp_authentication_required",
                403 => "mcp_access_denied",
                404 => "mcp_session_or_endpoint_not_found",
                429 => "mcp_rate_limited",
                _ => "mcp_http_failed",
            }
            .into());
        }
        if let Some(session) = response.headers().get("Mcp-Session-Id") {
            let session = session.to_str().map_err(|_| "mcp_session_invalid")?;
            if session.is_empty()
                || session.len() > 256
                || !session.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
                || self.remote_id.as_ref().is_some_and(|old| old != session)
            {
                return Err("mcp_session_invalid".into());
            }
            self.remote_id = Some(session.into());
        }
        if value.get("id").is_none() {
            if response.status() != reqwest::StatusCode::ACCEPTED
                || response.content_length().is_some_and(|n| n > 0)
            {
                return Err("mcp_notification_unconfirmed".into());
            }
            return Ok((String::new(), Vec::new()));
        }
        read_response(response, value["id"].as_u64(), side_effect).await
    }
    pub async fn notify(&mut self, execution: &mut Execution, value: &Value) -> Result<(), String> {
        self.post(execution, value, false).await.map(|_| ())
    }
    pub fn negotiate(&mut self, protocol: &str) {
        self.protocol = Some(protocol.into());
    }
    pub fn fingerprint(&self, name: &str, version: &str) -> Result<String, String> {
        digest(&json!({"source":self.config,"serverName":name,"serverVersion":version}))
    }
}
async fn read_response(
    mut response: reqwest::Response,
    id: Option<u64>,
    side_effect: bool,
) -> Result<(String, Vec<u8>), String> {
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_owned();
    if !matches!(
        content_type.as_str(),
        "application/json" | "text/event-stream"
    ) {
        return Err("mcp_transport_not_supported".into());
    }
    if response.content_length().is_some_and(|n| n > LIMIT as u64) {
        return Err("mcp_response_too_large".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| {
        if side_effect {
            "mcp_tool_outcome_unconfirmed"
        } else {
            "mcp_stream_failed"
        }
    })? {
        if bytes.len() + chunk.len() > LIMIT {
            return Err("mcp_response_too_large".into());
        }
        bytes.extend_from_slice(&chunk);
        if content_type == "text/event-stream"
            && let Some(id) = id
            && (bytes.ends_with(b"\n\n") || bytes.ends_with(b"\r\n\r\n"))
        {
            match parse_response(&content_type, &bytes, id) {
                Ok(_) => break,
                Err(reason) if reason == "mcp_response_missing" => (),
                Err(reason) => return Err(reason),
            }
        }
    }
    Ok((content_type, bytes))
}
pub(super) fn bounded(value: &Value, max: usize) -> Result<&str, String> {
    value
        .as_str()
        .filter(|value| {
            !value.is_empty() && value.len() <= max && !value.chars().any(char::is_control)
        })
        .ok_or_else(|| "mcp_response_invalid".into())
}
pub(super) fn parse_response(content_type: &str, bytes: &[u8], id: u64) -> Result<Value, String> {
    let decode = |bytes: &[u8]| {
        serde_json::from_slice::<Value>(bytes).map_err(|_| "mcp_response_invalid".to_owned())
    };
    let messages = if content_type == "application/json" {
        vec![decode(bytes)?]
    } else {
        let text = std::str::from_utf8(bytes)
            .map_err(|_| "mcp_response_invalid")?
            .replace("\r\n", "\n");
        let mut messages = Vec::new();
        for event in text.split("\n\n") {
            let data = event
                .lines()
                .filter_map(|line| {
                    line.strip_prefix("data:")
                        .map(|v| v.strip_prefix(' ').unwrap_or(v))
                })
                .collect::<Vec<_>>()
                .join("\n");
            if !data.is_empty() {
                messages.push(decode(data.as_bytes())?);
            }
        }
        messages
    };
    let mut response = None;
    for message in messages {
        if message["jsonrpc"] != "2.0" {
            return Err("mcp_response_invalid".into());
        }
        if message.get("method").is_some() {
            if message.get("id").is_some() {
                return Err("mcp_server_requests_not_supported".into());
            }
            if message["method"] == "notifications/tools/list_changed" {
                return Err("mcp_catalog_changed".into());
            }
            continue;
        }
        if message["id"] != id || response.is_some() {
            return Err("mcp_response_id_mismatch".into());
        }
        if message.get("error").is_some() {
            return Err("mcp_remote_rpc_failed".into());
        }
        response = Some(
            message
                .get("result")
                .cloned()
                .ok_or("mcp_response_invalid")?,
        );
    }
    response.ok_or_else(|| "mcp_response_missing".into())
}

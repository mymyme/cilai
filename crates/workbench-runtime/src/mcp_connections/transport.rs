use super::{Execution, credential, local, remote};
use contexthub_protocol::{McpConnection, McpSource, McpToolDefinition};
use remote::{Evidence, bounded};
use serde_json::{Value, json};
use std::collections::BTreeSet;
const PROTOCOL: &str = "2025-06-18";
pub(super) enum Session {
    Remote(Box<remote::Session>),
    Local(Box<local::Session>),
}
impl Session {
    pub fn open(execution: &mut Execution, connection: &McpConnection) -> Result<Self, String> {
        match connection.config.source {
            McpSource::RemoteHttp { .. } => remote::Session::open(
                connection,
                credential(&execution.data_dir, connection)?,
                &super::credential_vault_root(&execution.data_dir)?,
            )
            .map(|session| Self::Remote(Box::new(session))),
            McpSource::LocalCommand { .. } => local::Session::open(execution, connection)
                .map(|session| Self::Local(Box::new(session))),
        }
    }
    pub async fn request(
        &mut self,
        execution: &mut Execution,
        method: &str,
        params: Value,
        side_effect: bool,
    ) -> Result<Value, String> {
        match self {
            Self::Remote(session) => {
                session
                    .request(execution, method, params, side_effect)
                    .await
            }
            Self::Local(session) => {
                session
                    .request(execution, method, params, side_effect)
                    .await
            }
        }
    }
    async fn notify(&mut self, execution: &mut Execution, value: &Value) -> Result<(), String> {
        match self {
            Self::Remote(session) => session.notify(execution, value).await,
            Self::Local(session) => session.notify(execution, value).await,
        }
    }
    fn negotiate(&mut self, protocol: &str) {
        if let Self::Remote(session) = self {
            session.negotiate(protocol);
        }
    }
    fn fingerprint(&self, name: &str, version: &str) -> Result<String, String> {
        match self {
            Self::Remote(session) => session.fingerprint(name, version),
            Self::Local(session) => session.fingerprint(name, version),
        }
    }
    pub async fn discover(&mut self, execution: &mut Execution) -> Result<Evidence, String> {
        let init=self.request(execution,"initialize",json!({"protocolVersion":PROTOCOL,"capabilities":{},"clientInfo":{"name":"cilai-personal-workbench","version":"0.1.0"}}),false).await?;
        let protocol = init["protocolVersion"]
            .as_str()
            .filter(|value| matches!(*value, "2025-03-26" | PROTOCOL))
            .ok_or("mcp_protocol_not_supported")?
            .to_owned();
        let name = bounded(&init["serverInfo"]["name"], 240)?.to_owned();
        let version = bounded(&init["serverInfo"]["version"], 240)?.to_owned();
        if !init["capabilities"]["tools"].is_object() {
            return Err("mcp_tools_not_supported".into());
        }
        self.negotiate(&protocol);
        self.notify(
            execution,
            &json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        )
        .await?;
        let mut tools = Vec::new();
        let mut cursor = None;
        let mut cursors = BTreeSet::new();
        for _ in 0..16 {
            let result = self
                .request(
                    execution,
                    "tools/list",
                    cursor
                        .as_ref()
                        .map_or_else(|| json!({}), |cursor| json!({"cursor":cursor})),
                    false,
                )
                .await?;
            let page = result["tools"].as_array().ok_or("mcp_catalog_invalid")?;
            if page.len() + tools.len() > 256 {
                return Err("mcp_catalog_too_large".into());
            }
            for tool in page {
                let name = bounded(&tool["name"], 128)?;
                if !name
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.'))
                    || tools.iter().any(|old: &McpToolDefinition| old.name == name)
                    || !tool["inputSchema"].is_object()
                    || tool["inputSchema"]["type"] != "object"
                {
                    return Err("mcp_catalog_invalid".into());
                }
                let description = tool
                    .get("description")
                    .filter(|v| !v.is_null())
                    .map(|v| {
                        v.as_str()
                            .filter(|v| {
                                v.len() <= 8192
                                    && !v
                                        .chars()
                                        .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
                            })
                            .map(str::to_owned)
                            .ok_or_else(|| "mcp_catalog_invalid".to_owned())
                    })
                    .transpose()?;
                if tool
                    .get("annotations")
                    .is_some_and(|value| !value.is_object())
                {
                    return Err("mcp_catalog_invalid".into());
                }
                tools.push(McpToolDefinition {
                    name: name.into(),
                    description,
                    input_schema: tool["inputSchema"].clone(),
                    annotations: tool.get("annotations").cloned(),
                });
            }
            cursor = result
                .get("nextCursor")
                .filter(|value| !value.is_null())
                .map(|v| bounded(v, 4096).map(str::to_owned))
                .transpose()?;
            if let Some(cursor) = &cursor {
                if !cursors.insert(cursor.clone()) {
                    return Err("mcp_catalog_cursor_repeated".into());
                }
            } else {
                tools.sort_by(|a, b| a.name.cmp(&b.name));
                return Ok(Evidence {
                    protocol,
                    name: name.clone(),
                    version: version.clone(),
                    fingerprint: self.fingerprint(&name, &version)?,
                    tools,
                });
            }
        }
        Err("mcp_catalog_too_large".into())
    }
}

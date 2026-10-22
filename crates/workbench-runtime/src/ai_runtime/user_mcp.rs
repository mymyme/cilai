use super::{AiRuntimeManager, ApplicationWriteToolScope, LocalToolDisclosure};
use crate::application_tool_adapter::ScopeValidator;
use crate::mcp_connections::{ToolApproval, broker};
use contexthub_protocol::{McpAuthMode, McpSource, McpToolSnapshot, ModelPurpose};
use serde_json::json;
use std::sync::Arc;

impl AiRuntimeManager {
    pub(super) fn selected_user_mcp_snapshot(
        &self,
        purpose: &ModelPurpose,
        allowed: bool,
    ) -> Result<Option<McpToolSnapshot>, String> {
        if !allowed {
            return Ok(None);
        }
        let purpose = match purpose {
            ModelPurpose::Foreground => contexthub_protocol::McpPurpose::Foreground,
            ModelPurpose::Automation => contexthub_protocol::McpPurpose::Automation,
            _ => return Err("mcp_purpose_not_authorized".into()),
        };
        let snapshot = self.personal_mcp_selected_snapshot(purpose)?;
        Ok((!snapshot.connections.is_empty()).then_some(snapshot))
    }

    pub(super) fn start_user_mcp_broker(
        &self,
        run_id: &str,
        snapshot: McpToolSnapshot,
        validate: ScopeValidator,
    ) -> Result<Arc<broker::McpBroker>, String> {
        let manager = self.clone();
        let approval_run_id = run_id.to_owned();
        let approval_validate = Arc::clone(&validate);
        let approval: ToolApproval = Arc::new(move |connection, tool, arguments, execute| {
            approval_validate()?;
            let scope = ApplicationWriteToolScope {
                application_id: connection.connection_id.clone(),
                application_name: connection.config.name.clone(),
                dynamic_name: format!(
                    "user_mcp_{}",
                    super::sha256_hex(&format!("{}:{}", connection.connection_id, tool.name))
                ),
                tool_name: tool.name.clone(),
                title: tool.name.clone(),
                description: tool.description.clone().unwrap_or_default(),
                input_schema: tool.input_schema.clone(),
                risk_level: "medium".into(),
                timeout_ms: Some(120_000),
                confirmation_view: Some(confirmation_view(&tool.input_schema, arguments)),
            };
            let disclosure = LocalToolDisclosure {
                network_access: match &connection.config.source {
                    McpSource::RemoteHttp { .. } => true,
                    McpSource::LocalCommand { permissions, .. } => permissions.network,
                },
                credential_labels: if connection.config.auth_mode == McpAuthMode::None {
                    vec![]
                } else {
                    vec!["MCP 服务凭据".into()]
                },
                user_mcp: true,
            };
            let confirmed_execute = |final_arguments: &serde_json::Value| {
                approval_validate()?;
                let output = execute(final_arguments)?;
                Ok(
                    json!({"status":if output["isError"]==true {"failed"} else {"succeeded"},"result":output}),
                )
            };
            let receipt = manager.confirm_and_invoke_local_tool_with_disclosure(
                &approval_run_id,
                &scope,
                arguments,
                &confirmed_execute,
                Some(&disclosure),
            )?;
            receipt
                .get("result")
                .cloned()
                .ok_or_else(|| "mcp_tool_result_invalid".into())
        });
        broker::start(self.clone(), snapshot, approval, validate).map(Arc::new)
    }
}

fn confirmation_view(
    schema: &serde_json::Value,
    arguments: &serde_json::Value,
) -> serde_json::Value {
    let fields = arguments.as_object().into_iter().flat_map(|arguments| arguments.iter()).map(|(name, value)| {
        let property = &schema["properties"][name];
        json!({
            "name":name,
            "label":property["title"].as_str().unwrap_or(name),
            "editable":schema["properties"].get(name).is_some() && (value.is_string() || value.is_number() || value.is_boolean())
        })
    }).collect::<Vec<_>>();
    json!({"reviewMode":"editableProposal","submitLabel":"确认执行","fields":fields})
}

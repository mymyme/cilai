//! 本轮工作归类经专用工具提交，回复正文保持自然语言流式输出。
//!
//! 部分模型适配把结构化输出实现为全程约束解码，会压制工具调用；把整段回复放进 JSON 又会让
//! 模型照抄历史信封。归类字段改走函数调用（与应用结果提交协议同一模式），工作台在回合结束后
//! 以最终回复正文补入 `reply`，拼回原有信封，沿用同一 Schema 校验与解析。
use crate::app_bridge::validate_tool_arguments;
use serde_json::{Map, Value, json};
use std::sync::{Arc, Mutex};

pub(crate) const CLASSIFICATION_TOOL: &str = "cilai_submit_turn_classification";
/// 提交时尚无最终正文，以占位正文跑完整信封校验，使条件规则错误当场回给模型修正。
const PENDING_REPLY: &str = "（最终回复正文由工作台在回合结束后填入）";

type EnvelopeValidator = Arc<dyn Fn(&str) -> Result<(), String> + Send + Sync>;

/// 本轮归类提交的协议与已接收结果；一次运行一份。
pub(crate) struct ClassificationSubmission {
    input_schema: Value,
    validate: EnvelopeValidator,
    accepted: Mutex<Option<Map<String, Value>>>,
}

impl ClassificationSubmission {
    /// 由最终回复 Schema 派生工具参数：去掉由工作台填写的 `reply`，保留其余字段与条件约束。
    pub(crate) fn new(output_schema: &Value, validate: EnvelopeValidator) -> Result<Self, String> {
        let mut input_schema = output_schema.clone();
        let object = input_schema
            .as_object_mut()
            .ok_or("classification_schema_invalid")?;
        if let Some(properties) = object.get_mut("properties").and_then(Value::as_object_mut) {
            properties.remove("reply");
        }
        if let Some(required) = object.get_mut("required").and_then(Value::as_array_mut) {
            required.retain(|name| name != "reply");
        }
        Ok(Self {
            input_schema,
            validate,
            accepted: Mutex::new(None),
        })
    }

    pub(crate) fn tool(&self) -> Value {
        // 动态工具声明只携带基础结构；条件约束在接收时完整校验，失败原因回给模型修正。
        let mut declared = self.input_schema.clone();
        if let Some(object) = declared.as_object_mut() {
            object.remove("allOf");
        }
        json!({
            "type": "function",
            "name": CLASSIFICATION_TOOL,
            "description": "提交本轮工作归类。完成用户要求的实际操作后调用一次，再用自然语言输出给用户的最终回复；回复正文不要包含 JSON、nonce 或归类字段。提交失败时按返回原因修正后重新提交。",
            "inputSchema": declared
        })
    }

    /// 处理归类提交请求；不属于本工具的消息返回 `Ok(false)`。
    pub(crate) fn handle(
        &self,
        message: &Value,
        thread_id: &str,
        turn_id: &str,
        send: &mut impl FnMut(&Value) -> Result<(), String>,
    ) -> Result<bool, String> {
        if message.get("method").and_then(Value::as_str) != Some("item/tool/call")
            || message.pointer("/params/tool").and_then(Value::as_str) != Some(CLASSIFICATION_TOOL)
        {
            return Ok(false);
        }
        let id = message
            .get("id")
            .ok_or("classification_request_id_missing")?;
        let submit = || -> Result<Map<String, Value>, String> {
            if message.pointer("/params/threadId").and_then(Value::as_str) != Some(thread_id)
                || message.pointer("/params/turnId").and_then(Value::as_str) != Some(turn_id)
            {
                return Err("归类不属于当前运行".into());
            }
            let args = message.pointer("/params/arguments").ok_or("缺少归类参数")?;
            validate_tool_arguments(&self.input_schema, args)?;
            let fields = args.as_object().cloned().ok_or("归类参数必须是对象")?;
            (self.validate)(&envelope_with_reply(fields.clone(), PENDING_REPLY)?)?;
            Ok(fields)
        };
        let (success, text) = match submit() {
            Ok(fields) => {
                // 以最后一次成功提交为准，模型可在完成更多操作后修正归类。
                *self
                    .accepted
                    .lock()
                    .map_err(|_| "classification_state_unavailable")? = Some(fields);
                (
                    true,
                    "归类已提交。现在用自然语言输出本轮给用户的最终回复，不要输出 JSON。"
                        .to_owned(),
                )
            }
            Err(error) => (false, format!("归类未接收：{error}")),
        };
        send(
            &json!({"id": id, "result": {"success": success, "contentItems": [{"type": "inputText", "text": text}]}}),
        )?;
        Ok(true)
    }

    /// 已提交归类时，以最终回复正文补入 `reply` 拼回信封；未提交返回 `None`，按旧信封解析。
    pub(crate) fn envelope(&self, reply: &str) -> Result<Option<String>, String> {
        let accepted = self
            .accepted
            .lock()
            .map_err(|_| "classification_state_unavailable")?
            .clone();
        accepted
            .map(|fields| envelope_with_reply(fields, reply))
            .transpose()
    }
}

fn envelope_with_reply(mut fields: Map<String, Value>, reply: &str) -> Result<String, String> {
    fields.insert("reply".into(), Value::String(reply.to_owned()));
    serde_json::to_string(&fields).map_err(|error| error.to_string())
}

#[cfg(test)]
#[path = "turn_classification_tests.rs"]
mod tests;

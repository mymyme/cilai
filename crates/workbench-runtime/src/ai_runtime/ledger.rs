use super::ConversationMessage;
use serde_json::{Value, json};
use std::collections::BTreeMap;

type Key = (String, String);

#[derive(Clone, Debug)]
struct CachedMessage {
    source: ConversationMessage,
    value: Value,
    /// 账本中该消息记录与 `value`、此位置一致时记下位置，供下次保存跳过比较。
    confirmed_position: Option<usize>,
}

#[derive(Clone, Debug, Default)]
pub(super) struct Baseline {
    records: BTreeMap<Key, Value>,
    /// 按消息缓存的账本投影（去除正文）；消息原文未变时不再重复序列化与计算摘要。
    message_cache: BTreeMap<String, CachedMessage>,
}

fn message_value(message: &ConversationMessage) -> Result<Value, String> {
    let mut value = serde_json::to_value(message).map_err(|_| "ai_ledger_record_invalid")?;
    let object = value.as_object_mut().ok_or("ai_ledger_record_invalid")?;
    object.remove("content");
    object.insert("contentBytes".into(), json!(message.content.trim().len()));
    object.insert(
        "contentDigest".into(),
        super::sha256_hex(&message.content).into(),
    );
    Ok(value)
}

pub(super) struct Pending {
    pub request: Value,
}

fn string<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value[key]
        .as_str()
        .filter(|v| !v.is_empty())
        .ok_or_else(|| format!("ai_ledger_missing:{key}"))
}

fn add(
    records: &mut BTreeMap<Key, Value>,
    collection: &str,
    id: &str,
    position: usize,
    value: Value,
) -> Result<(), String> {
    let mut record = json!({"id":id,"position":position});
    record["value"] = value;
    if records
        .insert((collection.into(), id.into()), record)
        .is_some()
    {
        return Err("ai_ledger_duplicate_record".into());
    }
    Ok(())
}

#[expect(clippy::too_many_lines, reason = "每类业务条目的映射规则集中校验")]
fn projection(state: &Value) -> Result<BTreeMap<Key, Value>, String> {
    let mut records = BTreeMap::new();
    for (collection, key) in [
        ("conversations", "conversationId"),
        ("messages", "messageId"),
        ("runs", "runId"),
    ] {
        for (position, source) in state[collection]
            .as_array()
            .ok_or("ai_ledger_state_invalid")?
            .iter()
            .enumerate()
        {
            let id = string(source, key)?;
            let mut value = source.clone();
            let object = value.as_object_mut().ok_or("ai_ledger_record_invalid")?;
            match collection {
                "conversations" => {
                    object.remove("title");
                    object.remove("archived");
                    if let Some(snapshot) = object
                        .get_mut("applicationHandoff")
                        .and_then(|source| source.get_mut("runSnapshot"))
                        .and_then(Value::as_object_mut)
                    {
                        snapshot.remove("events");
                    }
                }
                "messages" => {
                    let body = object
                        .remove("content")
                        .ok_or("ai_ledger_message_body_missing")?;
                    object.insert(
                        "contentBytes".into(),
                        json!(
                            body.as_str()
                                .ok_or("ai_ledger_message_body_invalid")?
                                .trim()
                                .len()
                        ),
                    );
                    object.insert(
                        "contentDigest".into(),
                        super::sha256_hex(body.as_str().ok_or("ai_ledger_message_body_invalid")?)
                            .into(),
                    );
                }
                "runs" => {
                    let outcome = source["events"].as_array().and_then(|events| {
                        events.iter().rev().find(|event| {
                            event["phase"] == "reply"
                                && event["kind"] == "mcp"
                                && event["eventType"] == "item.completed"
                        })
                    });
                    object.insert(
                        "latestToolOutcome".into(),
                        outcome.map_or(
                            Value::Null,
                            |event| json!({"status":event["status"],"summary":event["summary"]}),
                        ),
                    );
                    for field in [
                        "output",
                        "partialOutput",
                        "events",
                        "logTail",
                        "previousRevisions",
                        "regenerationDraft",
                    ] {
                        object.remove(field);
                    }
                }
                _ => unreachable!(),
            }
            add(&mut records, collection, id, position, value)?;
        }
    }
    for (field, source) in state.as_object().ok_or("ai_ledger_state_invalid")? {
        if matches!(field.as_str(), "conversations" | "messages" | "runs") {
            continue;
        }
        match field.as_str() {
            "agents" | "permissionRequests" | "automationRules" | "inboxReadItemIds" => {
                let id_field = match field.as_str() {
                    "agents" => "agentId",
                    "permissionRequests" => "permissionRequestId",
                    "automationRules" => "automationRuleId",
                    _ => "",
                };
                for (position, value) in source
                    .as_array()
                    .ok_or("ai_ledger_entry_invalid")?
                    .iter()
                    .enumerate()
                {
                    let member = if id_field.is_empty() {
                        value.as_str().ok_or("ai_ledger_entry_invalid")?
                    } else {
                        string(value, id_field)?
                    };
                    let id = format!("{field}:{member}");
                    add(
                        &mut records,
                        "entries",
                        &id,
                        position,
                        json!({"entryId":id,"field":field,"member":member,"shape":"array","value":value}),
                    )?;
                }
            }
            "conversationSession" | "draftAttachments" => {
                for (member, value) in source.as_object().ok_or("ai_ledger_entry_invalid")? {
                    let id = format!("{field}:{member}");
                    add(
                        &mut records,
                        "entries",
                        &id,
                        0,
                        json!({"entryId":id,"field":field,"member":member,"shape":"map","value":value}),
                    )?;
                }
            }
            _ => {
                add(
                    &mut records,
                    "entries",
                    field,
                    0,
                    json!({"entryId":field,"field":field,"shape":"scalar","value":source}),
                )?;
            }
        }
    }
    Ok(records)
}

impl Baseline {
    pub(super) fn accept_automation_record(
        &mut self,
        record: &contexthub_protocol::PersonalAutomationRuleRecord,
    ) -> Result<(), String> {
        if record.position < 0
            || record.digest
                != crate::persistent_threads::digest(
                    &json!({"position":record.position,"value":record.value}),
                )
        {
            return Err("automation_rule_record_invalid".into());
        }
        self.records.insert(
            ("entries".into(), record.id.clone()),
            json!({"id":record.id,"position":record.position,"digest":record.digest,"value":record.value}),
        );
        Ok(())
    }

    pub(super) fn prepare_personal_notifications(&self, source: &Value) -> Result<Pending, String> {
        const FIELD: &str = "personalNotifications";
        if !source.is_object() {
            return Err("personal_notification_state_invalid".into());
        }
        let value = json!({"entryId":FIELD,"field":FIELD,"shape":"scalar","value":source});
        let key = ("entries".to_owned(), FIELD.to_owned());
        let previous = self.records.get(&key);
        let records = if previous.is_some_and(|old| old["position"] == 0 && old["value"] == value) {
            Vec::new()
        } else {
            vec![
                json!({"collection":"entries","id":FIELD,"position":0,"value":value,"expectedDigest":previous.map(|old| &old["digest"])}),
            ]
        };
        Ok(Pending {
            request: json!({"commandId":uuid::Uuid::new_v4().to_string(),"records":records,"pendingInputs":[],"deliveryInputs":[]}),
        })
    }
    pub(super) fn refresh_memory_entries(&mut self, current: &Self) {
        for field in ["memoryPreferences", "memoryEnableConfirmation"] {
            let key = ("entries".to_owned(), field.to_owned());
            if let Some(record) = current.records.get(&key) {
                self.records.insert(key, record.clone());
            } else {
                self.records.remove(&key);
            }
        }
    }
    pub(super) fn read(ledger: &Value) -> Result<(Self, Value), String> {
        let mut baseline = Self::default();
        let mut state = json!({"conversations":[],"messages":[],"runs":[],"agents":[],"permissionRequests":[],"automationRules":[],"inboxReadItemIds":[],"conversationSession":{},"draftAttachments":{}});
        for collection in ["conversations", "messages", "runs", "entries"] {
            for record in ledger[collection]
                .as_array()
                .ok_or("ai_ledger_read_invalid")?
            {
                let id = string(record, "id")?;
                string(record, "digest")?;
                if collection == "entries"
                    && id.starts_with(contexthub_protocol::MODEL_ENTRY_PREFIX)
                {
                    continue;
                }
                if baseline
                    .records
                    .insert((collection.into(), id.into()), record.clone())
                    .is_some()
                {
                    return Err("ai_ledger_duplicate_record".into());
                }
                let value = &record["value"];
                if collection != "entries" {
                    state[collection]
                        .as_array_mut()
                        .ok_or("ai_ledger_read_invalid")?
                        .push(value.clone());
                    continue;
                }
                let field = string(value, "field")?;
                match string(value, "shape")? {
                    "array" => state[field]
                        .as_array_mut()
                        .ok_or("ai_ledger_entry_invalid")?
                        .push(value["value"].clone()),
                    "map" => {
                        state[field][string(value, "member")?] = value["value"].clone();
                    }
                    "scalar" => {
                        state[field] = value["value"].clone();
                    }
                    _ => return Err("ai_ledger_entry_invalid".into()),
                }
            }
        }
        Ok((baseline, state))
    }

    pub(super) fn prepare(&self, state: &Value) -> Result<Pending, String> {
        let records = projection(state)?;
        let mut changes = Vec::new();
        let mut pending_inputs = Vec::new();
        let mut delivery_inputs = Vec::new();
        for ((collection, id), record) in &records {
            let previous = self.records.get(&(collection.clone(), id.clone()));
            if previous.is_some_and(|old| {
                old["position"] == record["position"] && old["value"] == record["value"]
            }) {
                continue;
            }
            changes.push(json!({"collection":collection,"id":id,"position":record["position"],"value":record["value"],"expectedDigest":previous.map(|v| &v["digest"])}));
            if collection == "messages" && record["value"]["role"] == "user" {
                let source = state["messages"]
                    .as_array()
                    .ok_or("ai_ledger_state_invalid")?
                    .iter()
                    .find(|v| v["messageId"] == *id)
                    .ok_or("ai_ledger_state_invalid")?;
                pending_inputs.push(json!({"messageId":id,"content":source["content"],"contentDigest":record["value"]["contentDigest"],"nativeContentDigest":crate::persistent_threads::digest(&json!({"content":source["content"],"attachments":source["attachments"],"referencedMaterials":source["referencedMaterials"]}))}));
            }
            if collection == "messages"
                && record["value"]["role"] == "assistant"
                && state["runs"].as_array().is_some_and(|runs| {
                    runs.iter().any(|run| {
                        run["runId"] == record["value"]["runId"] && run["status"] == "completed"
                    })
                })
            {
                let source = state["messages"]
                    .as_array()
                    .ok_or("ai_ledger_state_invalid")?
                    .iter()
                    .find(|v| v["messageId"] == *id)
                    .ok_or("ai_ledger_state_invalid")?;
                delivery_inputs.push(
                    json!({"messageId":id,"runId":source["runId"],"content":source["content"]}),
                );
            }
        }
        for ((collection, id), previous) in &self.records {
            if !records.contains_key(&(collection.clone(), id.clone())) {
                changes.push(json!({"collection":collection,"id":id,"position":0,"value":null,"expectedDigest":previous["digest"]}));
            }
        }
        Ok(Pending {
            request: json!({"commandId":uuid::Uuid::new_v4().to_string(),"records":changes,"pendingInputs":pending_inputs,"deliveryInputs":delivery_inputs}),
        })
    }

    /// 与 `prepare` 产出一致，但消息单独传入并按消息缓存投影：未改动的消息只做一次
    /// 结构比较，不再序列化正文、计算摘要或克隆，耗时不随消息正文体量增长。
    /// `state` 须由 `ledger_projection::state_without_messages` 生成。
    pub(super) fn prepare_cached(
        &mut self,
        state: &Value,
        messages: &[ConversationMessage],
    ) -> Result<Pending, String> {
        let records = projection(state)?;
        let mut changes = Vec::new();
        let mut pending_inputs = Vec::new();
        let mut delivery_inputs = Vec::new();
        for ((collection, id), record) in &records {
            let previous = self.records.get(&(collection.clone(), id.clone()));
            if previous.is_some_and(|old| {
                old["position"] == record["position"] && old["value"] == record["value"]
            }) {
                continue;
            }
            changes.push(json!({"collection":collection,"id":id,"position":record["position"],"value":record["value"],"expectedDigest":previous.map(|v| &v["digest"])}));
        }
        let mut cache = std::mem::take(&mut self.message_cache);
        let mut next = BTreeMap::new();
        for (position, message) in messages.iter().enumerate() {
            let id = message.message_id.as_str();
            if id.is_empty() {
                return Err("ai_ledger_missing:messageId".into());
            }
            let cached = match cache.remove(id) {
                Some(cached) if cached.source == *message => cached,
                _ => CachedMessage {
                    source: message.clone(),
                    value: message_value(message)?,
                    confirmed_position: None,
                },
            };
            let previous = self.records.get(&("messages".to_owned(), id.to_owned()));
            let mut cached = cached;
            // 已确认与账本记录一致的消息不再逐字段比较；账本记录只会经 `acknowledge` 变化。
            if cached.confirmed_position != Some(position) {
                cached.confirmed_position = previous
                    .filter(|old| {
                        old["position"] == json!(position) && old["value"] == cached.value
                    })
                    .map(|_| position);
            }
            if cached.confirmed_position.is_none() {
                changes.push(json!({"collection":"messages","id":id,"position":position,"value":cached.value,"expectedDigest":previous.map(|v| &v["digest"])}));
                if message.role == "user" {
                    pending_inputs.push(json!({"messageId":id,"content":message.content,"contentDigest":cached.value["contentDigest"],"nativeContentDigest":crate::persistent_threads::digest(&json!({"content":message.content,"attachments":message.attachments,"referencedMaterials":message.referenced_materials}))}));
                }
                if message.role == "assistant"
                    && state["runs"].as_array().is_some_and(|runs| {
                        runs.iter().any(|run| {
                            run["runId"] == json!(message.run_id) && run["status"] == "completed"
                        })
                    })
                {
                    delivery_inputs.push(
                        json!({"messageId":id,"runId":message.run_id,"content":message.content}),
                    );
                }
            }
            if next.insert(id.to_owned(), cached).is_some() {
                return Err("ai_ledger_duplicate_record".into());
            }
        }
        for ((collection, id), previous) in &self.records {
            let present = if collection == "messages" {
                next.contains_key(id)
            } else {
                records.contains_key(&(collection.clone(), id.clone()))
            };
            if !present {
                changes.push(json!({"collection":collection,"id":id,"position":0,"value":null,"expectedDigest":previous["digest"]}));
            }
        }
        self.message_cache = next;
        Ok(Pending {
            request: json!({"commandId":uuid::Uuid::new_v4().to_string(),"records":changes,"pendingInputs":pending_inputs,"deliveryInputs":delivery_inputs}),
        })
    }

    /// 只投影并比对会话草稿/队列（`conversationSession`）这一组条目，成本与对话历史长度无关。
    pub(super) fn prepare_session(
        &self,
        session: &BTreeMap<String, Value>,
    ) -> Result<Pending, String> {
        const FIELD: &str = "conversationSession";
        let prefix = format!("{FIELD}:");
        let mut records = BTreeMap::new();
        for (member, value) in session {
            let id = format!("{prefix}{member}");
            add(
                &mut records,
                "entries",
                &id,
                0,
                json!({"entryId":id,"field":FIELD,"member":member,"shape":"map","value":value}),
            )?;
        }
        let mut changes = Vec::new();
        for ((collection, id), record) in &records {
            let previous = self.records.get(&(collection.clone(), id.clone()));
            if previous.is_some_and(|old| {
                old["position"] == record["position"] && old["value"] == record["value"]
            }) {
                continue;
            }
            changes.push(json!({"collection":collection,"id":id,"position":record["position"],"value":record["value"],"expectedDigest":previous.map(|v| &v["digest"])}));
        }
        let start = ("entries".to_owned(), prefix.clone());
        for ((collection, id), previous) in self.records.range(start..) {
            if collection != "entries" || !id.starts_with(&prefix) {
                break;
            }
            if !records.contains_key(&(collection.clone(), id.clone())) {
                changes.push(json!({"collection":collection,"id":id,"position":0,"value":null,"expectedDigest":previous["digest"]}));
            }
        }
        Ok(Pending {
            request: json!({"commandId":uuid::Uuid::new_v4().to_string(),"records":changes,"pendingInputs":[],"deliveryInputs":[]}),
        })
    }

    pub(super) fn prepare_run(
        &self,
        run: &super::AgentRun,
        position: usize,
    ) -> Result<Pending, String> {
        let state =
            json!({"conversations":[],"messages":[],"runs":[super::ledger_projection::run(run)]});
        let mut projected = projection(&state)?;
        let key = ("runs".to_owned(), run.run_id.clone());
        let mut record = projected
            .remove(&key)
            .ok_or("ai_ledger_run_projection_missing")?;
        record["position"] = json!(position);
        let previous = self.records.get(&key);
        let records = if previous.is_some_and(|old| {
            old["position"] == record["position"] && old["value"] == record["value"]
        }) {
            Vec::new()
        } else {
            vec![
                json!({"collection":"runs","id":run.run_id,"position":position,"value":record["value"],"expectedDigest":previous.map(|old|&old["digest"])}),
            ]
        };
        Ok(Pending {
            request: json!({"commandId":uuid::Uuid::new_v4().to_string(),"records":records,"pendingInputs":[],"deliveryInputs":[]}),
        })
    }

    pub(super) fn acknowledge(&mut self, pending: &Pending, receipt: &Value) -> Result<(), String> {
        if receipt["commandId"] != pending.request["commandId"] {
            return Err("ai_ledger_receipt_mismatch".into());
        }
        let replies = receipt["records"]
            .as_array()
            .ok_or("ai_ledger_receipt_invalid")?;
        let requests = pending.request["records"]
            .as_array()
            .ok_or("ai_ledger_request_invalid")?;
        if replies.len() != requests.len() {
            return Err("ai_ledger_receipt_incomplete".into());
        }
        let mut changes = Vec::new();
        for (request, reply) in requests.iter().zip(replies) {
            if request["collection"] != reply["collection"] || request["id"] != reply["id"] {
                return Err("ai_ledger_receipt_mismatch".into());
            }
            let key = (
                string(request, "collection")?.to_owned(),
                string(request, "id")?.to_owned(),
            );
            let record = if request["value"].is_null() {
                if !reply["digest"].is_null() {
                    return Err("ai_ledger_receipt_mismatch".into());
                }
                None
            } else {
                Some(
                    json!({"id":request["id"],"position":request["position"],"value":request["value"],"digest":string(reply,"digest")?}),
                )
            };
            changes.push((key, record));
        }
        for (key, record) in changes {
            if key.0 == "messages" {
                let confirmed = record.as_ref().and_then(|record| {
                    let cached = self.message_cache.get(&key.1)?;
                    let position = record["position"]
                        .as_u64()
                        .and_then(|p| usize::try_from(p).ok())?;
                    (cached.value == record["value"]).then_some(position)
                });
                if let Some(cached) = self.message_cache.get_mut(&key.1) {
                    cached.confirmed_position = confirmed;
                }
            }
            if let Some(record) = record {
                self.records.insert(key, record);
            } else {
                self.records.remove(&key);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn model_owned_entries_never_enter_projection_or_deletion_baselines() {
        let ledger = json!({"conversations":[],"messages":[],"runs":[],"entries":[
            {"id":"personalModel:connection:existing","position":0,"digest":"connection-digest","value":{"entryId":"personalModel:connection:existing","owner":"personal_model","kind":"connection","value":{"secretRef":{"service":"cn.cilai.personal.workbench.model","account":"profile:connection:1"}}}},
            {"id":"approvalReviewModel","position":0,"digest":"legacy-digest","value":{"entryId":"approvalReviewModel","field":"approvalReviewModel","shape":"scalar","value":null}}
        ]});
        let (mut baseline, mut projected) = Baseline::read(&ledger).unwrap();
        assert!(!projected.to_string().contains("personalModel:"));
        assert_eq!(baseline.records.len(), 1);
        assert_eq!(
            baseline.prepare(&projected).unwrap().request["records"],
            json!([])
        );
        projected["conversationSession"]["c"] = json!({"draft":"保留草稿"});
        for pending in [
            baseline.prepare(&projected).unwrap(),
            baseline.prepare_cached(&projected, &[]).unwrap(),
        ] {
            assert!(
                pending.request["records"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|v| !v["id"].as_str().unwrap().starts_with("personalModel:"))
            );
        }
        assert_eq!(ledger["entries"][0]["digest"], "connection-digest");
    }
    fn state() -> Value {
        json!({"conversations":[{"conversationId":"c","title":"线程名称","archived":false}],"messages":[{"messageId":"u","role":"user","content":"原文"},{"messageId":"a","role":"assistant","content":"回答"}],"runs":[{"runId":"r","output":"回答","partialOutput":"部分","events":[1],"logTail":["日志"],"previousRevisions":["版本"]}],"agents":[],"permissionRequests":[],"automationRules":[],"inboxReadItemIds":[],"conversationSession":{"c":{"draft":"尚未发送"}},"draftAttachments":{},"approvalReviewModel":null})
    }
    #[test]
    fn metadata_excludes_transcript_and_unaccepted_input_is_explicit() {
        let mut source = state();
        source["conversations"][0]["applicationHandoff"] = json!({"runSnapshot":{"modelId":"original-model","modelLabel":"原模型","startedAt":1,"completedAt":2,"events":[{"summary":"完整来源执行内容"}]}});
        let pending = Baseline::default().prepare(&source).unwrap();
        let encoded = pending.request["records"].to_string();
        for body in [
            "原文",
            "回答",
            "部分",
            "日志",
            "线程名称",
            "previousRevisions",
            "完整来源执行内容",
        ] {
            assert!(!encoded.contains(body));
        }
        assert!(encoded.contains("尚未发送"));
        assert!(encoded.contains("original-model"));
        assert!(encoded.contains("原模型"));
        assert_eq!(
            pending.request["pendingInputs"],
            json!([{"messageId":"u","content":"原文","contentDigest":super::super::sha256_hex("原文"),"nativeContentDigest":crate::persistent_threads::digest(&json!({"content":"原文","attachments":null,"referencedMaterials":null}))} ])
        );
    }
    #[test]
    fn replay_receipt_and_unchanged_content_produce_no_records() {
        let mut baseline = Baseline::default();
        let pending = baseline.prepare(&state()).unwrap();
        let receipt = json!({"commandId":pending.request["commandId"],"records":pending.request["records"].as_array().unwrap().iter().map(|r| json!({"collection":r["collection"],"id":r["id"],"digest":"stored-digest"})).collect::<Vec<_>>()});
        baseline.acknowledge(&pending, &receipt).unwrap();
        assert_eq!(
            baseline.prepare(&state()).unwrap().request["records"],
            json!([])
        );
        let mut edited = state();
        edited["conversationSession"]["c"]["draft"] = json!("新草稿");
        let pending = baseline.prepare(&edited).unwrap();
        assert_eq!(pending.request["records"].as_array().unwrap().len(), 1);
        assert_eq!(pending.request["records"][0]["id"], "conversationSession:c");
        assert_eq!(pending.request["pendingInputs"], json!([]));
    }
    #[test]
    fn cached_preparation_matches_full_projection_and_reuses_unchanged_messages() {
        let messages: Vec<ConversationMessage> = serde_json::from_value(json!([
            {"messageId":"u","conversationId":"c","role":"user","content":"原文","runId":null,"createdAt":1},
            {"messageId":"a","conversationId":"c","role":"assistant","content":"回答","runId":"r","createdAt":2}
        ])).unwrap();
        let mut source = state();
        source["messages"] = serde_json::to_value(&messages).unwrap();
        let expected = Baseline::default().prepare(&source).unwrap();
        source["messages"] = json!([]);
        let mut baseline = Baseline::default();
        let actual = baseline.prepare_cached(&source, &messages).unwrap();
        let sorted = |p: &Pending| {
            let mut records = p.request["records"].as_array().unwrap().clone();
            records.sort_by_key(|r| (r["collection"].to_string(), r["id"].to_string()));
            records
        };
        assert_eq!(sorted(&actual), sorted(&expected));
        assert_eq!(
            actual.request["pendingInputs"],
            expected.request["pendingInputs"]
        );
        let receipt = json!({"commandId":actual.request["commandId"],"records":actual.request["records"].as_array().unwrap().iter().map(|r| json!({"collection":r["collection"],"id":r["id"],"digest":"d"})).collect::<Vec<_>>()});
        baseline.acknowledge(&actual, &receipt).unwrap();
        assert_eq!(
            baseline.prepare_cached(&source, &messages).unwrap().request["records"],
            json!([])
        );
        let mut edited = messages.clone();
        edited[1].content = "新回答".into();
        let changed = baseline.prepare_cached(&source, &edited).unwrap();
        assert_eq!(changed.request["records"].as_array().unwrap().len(), 1);
        assert_eq!(changed.request["records"][0]["id"], "a");
        let removed = baseline.prepare_cached(&source, &edited[..1]).unwrap();
        assert!(
            removed.request["records"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["id"] == "a" && r["value"].is_null())
        );
    }
    #[test]
    fn session_only_preparation_matches_full_projection_for_session_changes() {
        let mut baseline = Baseline::default();
        let pending = baseline.prepare(&state()).unwrap();
        let receipt = json!({"commandId":pending.request["commandId"],"records":pending.request["records"].as_array().unwrap().iter().map(|r| json!({"collection":r["collection"],"id":r["id"],"digest":"d"})).collect::<Vec<_>>()});
        baseline.acknowledge(&pending, &receipt).unwrap();
        let mut session: BTreeMap<String, Value> =
            serde_json::from_value(state()["conversationSession"].clone()).unwrap();
        assert_eq!(
            baseline.prepare_session(&session).unwrap().request["records"],
            json!([])
        );
        session.insert("c".into(), json!({"draft":"新草稿"}));
        session.insert("d".into(), json!({"draft":"另一条"}));
        let ids = |p: &Pending| -> Vec<Value> {
            p.request["records"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| r["id"].clone())
                .collect()
        };
        assert_eq!(
            ids(&baseline.prepare_session(&session).unwrap()),
            vec![
                json!("conversationSession:c"),
                json!("conversationSession:d")
            ]
        );
        session.remove("c");
        let removal = baseline.prepare_session(&session).unwrap();
        let records = removal.request["records"].as_array().unwrap();
        let deleted = records
            .iter()
            .find(|r| r["id"] == "conversationSession:c")
            .unwrap();
        assert!(deleted["value"].is_null());
    }
}

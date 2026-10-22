use crate::codex_harness::{
    ApplicationWriteCallback, ApplicationWriteToolScope, application_write_model_schema,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct AdapterSlot {
    state: tokio::sync::watch::Sender<Option<Result<Arc<ApplicationToolAdapter>, String>>>,
}

impl Default for AdapterSlot {
    fn default() -> Self {
        Self {
            state: tokio::sync::watch::channel(None).0,
        }
    }
}

impl AdapterSlot {
    pub(crate) fn complete(&self, result: Result<Arc<ApplicationToolAdapter>, String>) {
        self.state.send_replace(Some(result));
    }

    pub(crate) fn current(&self) -> Option<Arc<ApplicationToolAdapter>> {
        self.state
            .borrow()
            .as_ref()
            .and_then(|result| result.as_ref().ok())
            .cloned()
    }

    pub(crate) async fn wait_ready(&self) -> Result<Arc<ApplicationToolAdapter>, String> {
        let mut state = self.state.subscribe();
        loop {
            if let Some(result) = state.borrow_and_update().clone() {
                return result;
            }
            state
                .changed()
                .await
                .map_err(|_| "application_tool_state_closed".to_owned())?;
        }
    }

    pub(crate) fn preparation_guard(&self) -> AdapterPreparationGuard {
        AdapterPreparationGuard(self.clone())
    }
}

pub(crate) struct AdapterPreparationGuard(AdapterSlot);

impl Drop for AdapterPreparationGuard {
    fn drop(&mut self) {
        // 准备线程异常退出也必须唤醒正在等 tools/list 的请求，不能永久等待。
        self.0.state.send_if_modified(|state| {
            if state.is_none() {
                *state = Some(Err("application_tool_preparation_aborted".to_owned()));
                true
            } else {
                false
            }
        });
    }
}
pub(crate) type ScopeValidator = Arc<dyn Fn() -> Result<(), String> + Send + Sync>;

enum Target {
    Write(Box<ApplicationWriteToolScope>),
    Personal(crate::codex_harness::PersonalToolsCallback),
    ReportResult(crate::report_result::ReportResultContract),
}

struct Entry {
    identity: String,
    definition: Value,
    target: Target,
    /// 员工在执行过程里看到的名称；版本名只供模型与契约校验使用
    label: String,
}

pub(crate) struct ApplicationToolAdapter {
    tools: BTreeMap<String, Entry>,
    write: Option<ApplicationWriteCallback>,
    validate: ScopeValidator,
    report_result: Option<crate::report_result::ReportResultContract>,
}

pub(crate) fn versioned_name(identity: &str, definition: &Value) -> String {
    let logical = format!("{:x}", Sha256::digest(identity.as_bytes()));
    let contract = format!("{:x}", Sha256::digest(definition.to_string().as_bytes()));
    format!("a_{}_{}", &logical[..20], &contract[..20])
}

impl ApplicationToolAdapter {
    pub(crate) fn new_local(
        write: Option<ApplicationWriteCallback>,
        validate: ScopeValidator,
    ) -> Self {
        Self {
            tools: BTreeMap::new(),
            write,
            validate,
            report_result: None,
        }
    }

    fn insert(
        &mut self,
        identity: String,
        mut definition: Value,
        target: Target,
        label: String,
    ) -> Result<(), String> {
        if !definition["inputSchema"].is_object() || !definition.is_object() {
            return Err("application_tool_contract_invalid".to_owned());
        }
        definition
            .as_object_mut()
            .expect("validated object")
            .remove("name");
        let contract = match &target {
            Target::Personal(_) | Target::ReportResult(_) => definition.clone(),
            Target::Write(tool) => {
                json!({"definition":definition,"risk":tool.risk_level,"confirmationView":tool.confirmation_view,"inputSchema":tool.input_schema})
            }
        };
        let name = versioned_name(&identity, &contract);
        definition["name"] = json!(name);
        if let Some(previous) = self.tools.get(&name) {
            if previous.identity != identity || previous.definition != definition {
                return Err("application_tool_identity_collision".to_owned());
            }
            return Err("application_tool_identity_duplicate".to_owned());
        }
        self.tools.insert(
            name,
            Entry {
                identity,
                definition,
                target,
                label,
            },
        );
        Ok(())
    }

    pub(crate) fn add_personal(
        &mut self,
        definition: Value,
        callback: crate::codex_harness::PersonalToolsCallback,
    ) -> Result<(), String> {
        self.insert(
            "personal-capability\nentry".into(),
            definition,
            Target::Personal(callback),
            "个人能力".into(),
        )
    }

    pub(crate) fn add_report_materials(
        &mut self,
        definition: Value,
        callback: crate::codex_harness::PersonalToolsCallback,
    ) -> Result<(), String> {
        self.insert(
            "report-materials\nread".into(),
            definition,
            Target::Personal(callback),
            "读取获准本地材料".into(),
        )
    }

    pub(crate) fn add_write(&mut self, tool: ApplicationWriteToolScope) -> Result<(), String> {
        if self.write.is_none() || !matches!(tool.risk_level.as_str(), "write" | "highRisk") {
            return Err("application_write_handler_missing".to_owned());
        }
        let definition = json!({
            "title": tool.title, "description": tool.description,
            "inputSchema": application_write_model_schema(&tool),
            "annotations": { "readOnlyHint": false, "destructiveHint": true },
        });
        let title = if tool.title.trim().is_empty() {
            &tool.tool_name
        } else {
            &tool.title
        };
        let label = format!("{} · {title}", tool.application_name);
        self.insert(
            format!("{}\n{}", tool.application_id, tool.tool_name),
            definition,
            Target::Write(Box::new(tool)),
            label,
        )
    }

    pub(crate) fn add_report_result(
        &mut self,
        identity: &str,
        contract: crate::report_result::ReportResultContract,
    ) -> Result<(), String> {
        if self.report_result.is_some() {
            return Err("report_result_contract_duplicate".into());
        }
        self.insert(
            identity.into(),
            contract.tool_definition(),
            Target::ReportResult(contract.clone()),
            "提交报告核实结果".into(),
        )?;
        self.report_result = Some(contract);
        Ok(())
    }

    /// 版本名到员工可读名称的对照，供执行过程和步骤详情显示
    pub(crate) fn labels(&self) -> BTreeMap<String, String> {
        self.tools
            .iter()
            .map(|(name, entry)| (name.clone(), entry.label.clone()))
            .collect()
    }

    pub(crate) fn list(&self) -> Result<Value, String> {
        (self.validate)()?;
        Ok(
            json!({ "tools": self.tools.values().map(|entry| entry.definition.clone()).collect::<Vec<_>>() }),
        )
    }

    pub(crate) fn call(&self, name: &str, arguments: Value) -> Result<Value, String> {
        (self.validate)()?;
        let entry = self
            .tools
            .get(name)
            .ok_or_else(|| "tool_scope_denied_or_contract_stale".to_owned())?;
        if !arguments.is_object() {
            return Err("application_tool_arguments_invalid".to_owned());
        }
        match &entry.target {
            Target::ReportResult(contract) => contract.submit(&arguments),
            Target::Personal(callback) => {
                let result = callback(arguments)?;
                Ok(
                    json!({"content":[{"type":"text","text":result.to_string()}],"structuredContent":result}),
                )
            }
            Target::Write(tool) => {
                let handler = self
                    .write
                    .as_ref()
                    .ok_or_else(|| "application_write_handler_missing".to_owned())?;
                let result = handler(tool, arguments)?;
                Ok(
                    json!({"content":[{"type":"text","text":result.to_string()}],"structuredContent":result}),
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::FutureExt as _;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::time::Duration;

    #[test]
    fn preparing_adapter_waits_for_real_definitions_and_retains_ready_result() {
        let slot = AdapterSlot::default();
        assert!(slot.wait_ready().now_or_never().is_none());
        let guard = slot.preparation_guard();
        let adapter = Arc::new(ApplicationToolAdapter::new_local(None, Arc::new(|| Ok(()))));
        slot.complete(Ok(adapter.clone()));
        drop(guard);
        let ready = slot.wait_ready().now_or_never().unwrap().unwrap();
        assert!(Arc::ptr_eq(&ready, &adapter));
    }

    #[test]
    fn failed_or_aborted_preparation_wakes_waiters_with_error() {
        let slot = AdapterSlot::default();
        slot.complete(Err("snapshot_failed".into()));
        assert_eq!(
            slot.wait_ready().now_or_never().unwrap().err().unwrap(),
            "snapshot_failed"
        );
        let aborted = AdapterSlot::default();
        drop(aborted.preparation_guard());
        assert_eq!(
            aborted.wait_ready().now_or_never().unwrap().err().unwrap(),
            "application_tool_preparation_aborted"
        );
    }

    #[test]
    fn report_submission_uses_the_current_scoped_adapter_without_business_write_or_remote_call() {
        let allowed = Arc::new(AtomicBool::new(true));
        let current = allowed.clone();
        let mut adapter = ApplicationToolAdapter::new_local(
            None,
            Arc::new(move || {
                if current.load(Ordering::SeqCst) {
                    Ok(())
                } else {
                    Err("tool_scope_denied".into())
                }
            }),
        );
        let contract: crate::report_result::ReportResultContract = serde_json::from_value(json!({"schemaVersion":2,"clientMessageId":"client","inputDigest":"digest","reportId":"00000000-0000-0000-0000-000000000001","versionId":"00000000-0000-0000-0000-000000000002"})).unwrap();
        contract.add_to_adapter(&mut adapter).unwrap();
        let tools = adapter.list().unwrap();
        assert_eq!(tools["tools"].as_array().unwrap().len(), 1);
        assert_eq!(tools["tools"][0]["name"], contract.tool_name());
        let arguments = json!({"schemaVersion":2,"clientMessageId":"client","inputDigest":"digest","reportId":"00000000-0000-0000-0000-000000000001","versionId":"00000000-0000-0000-0000-000000000002","result":{"answer":"材料不足，未修改","html":null,"edits":[]}});
        let mut wrong = arguments.clone();
        wrong["clientMessageId"] = json!("previous");
        assert!(adapter.call(&contract.tool_name(), wrong).is_err());
        assert!(contract.complete("普通文字").is_err());
        assert_eq!(
            adapter.call(&contract.tool_name(), arguments).unwrap()["structuredContent"]["accepted"],
            true
        );
        allowed.store(false, Ordering::SeqCst);
        assert!(adapter.call(&contract.tool_name(), json!({})).is_err());
    }
    #[test]
    fn personal_entry_uses_each_turn_scope_and_never_calls_a_remote_gateway() {
        let allowed = Arc::new(AtomicBool::new(true));
        let current = allowed.clone();
        let mut adapter = ApplicationToolAdapter::new_local(
            None,
            Arc::new(move || {
                if current.load(Ordering::SeqCst) {
                    Ok(())
                } else {
                    Err("run_finished".into())
                }
            }),
        );
        adapter
            .add_personal(
                json!({"name":"personal","description":"个人能力","inputSchema":{"type":"object"}}),
                Arc::new(|value| Ok(json!({"action":value["action"],"items":[]}))),
            )
            .unwrap();
        let name = adapter.list().unwrap()["tools"][0]["name"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_eq!(
            adapter.call(&name, json!({"action":"list"})).unwrap()["structuredContent"]["action"],
            "list"
        );
        allowed.store(false, Ordering::SeqCst);
        assert!(adapter.list().is_err());
        assert_eq!(adapter.call(&name, json!({})).unwrap_err(), "run_finished");
        let next = ApplicationToolAdapter::new_local(None, Arc::new(|| Ok(())));
        assert!(next.call(&name, json!({})).is_err());
    }

    fn tool() -> ApplicationWriteToolScope {
        ApplicationWriteToolScope {
            application_id: "app".into(),
            application_name: "应用".into(),
            dynamic_name: "old".into(),
            tool_name: "create".into(),
            title: "创建".into(),
            description: "创建对象".into(),
            input_schema: json!({"type":"object","properties":{"name":{"type":"string"}},"required":["name"]}),
            risk_level: "write".into(),
            timeout_ms: None,
            confirmation_view: None,
        }
    }

    #[test]
    fn display_labels_use_application_and_tool_titles() {
        let callback: ApplicationWriteCallback = Arc::new(|_, _| Ok(json!({"ok":true})));
        let mut adapter = ApplicationToolAdapter::new_local(Some(callback), Arc::new(|| Ok(())));
        adapter.add_write(tool()).unwrap();
        assert_eq!(
            adapter.labels().into_values().collect::<Vec<_>>(),
            vec!["应用 · 创建"]
        );
        assert!(adapter.labels().keys().all(|name| name.starts_with("a_")));
    }

    #[test]
    fn definitions_stay_stable_across_runs_but_change_with_contract() {
        let callback: ApplicationWriteCallback = Arc::new(|_, _| Ok(json!({"ok":true})));
        let build = |tool| {
            let mut adapter =
                ApplicationToolAdapter::new_local(Some(callback.clone()), Arc::new(|| Ok(())));
            adapter.add_write(tool).unwrap();
            adapter
        };
        let first = build(tool());
        let second = build(tool());
        assert_eq!(first.list().unwrap(), second.list().unwrap());
        let mut changed = tool();
        changed.input_schema["required"] = json!(["name", "date"]);
        let third = build(changed);
        let old = first.list().unwrap()["tools"][0]["name"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(third.call(&old, json!({"name":"n"})).is_err());
    }

    #[test]
    fn writes_wait_for_the_host_decision_and_rejection_or_expiry_cannot_commit() {
        let writes = Arc::new(AtomicUsize::new(0));
        let committed = Arc::clone(&writes);
        let (requested_tx, requested_rx) = std::sync::mpsc::channel();
        let (decision_tx, decision_rx) = std::sync::mpsc::channel::<Result<(), String>>();
        let decisions = std::sync::Mutex::new(decision_rx);
        let mut adapter = ApplicationToolAdapter::new_local(
            Some(Arc::new(move |tool, args| {
                assert_eq!(tool.tool_name, "create");
                assert_eq!(args, json!({"name":"n"}));
                requested_tx.send(()).unwrap();
                decisions.lock().unwrap().recv().unwrap()?;
                committed.fetch_add(1, Ordering::SeqCst);
                Ok(json!({"receiptId":"confirmed"}))
            })),
            Arc::new(|| Ok(())),
        );
        adapter.add_write(tool()).unwrap();
        let definition = adapter.list().unwrap()["tools"][0].clone();
        assert_eq!(definition["annotations"]["readOnlyHint"], false);
        assert_eq!(definition["annotations"]["destructiveHint"], true);
        let name = definition["name"].as_str().unwrap().to_owned();
        let adapter = Arc::new(adapter);
        for decision in [
            Err("application_write_rejected".to_owned()),
            Err("application_write_confirmation_expired".to_owned()),
            Ok(()),
        ] {
            let worker_adapter = Arc::clone(&adapter);
            let worker_name = name.clone();
            let (result_tx, result_rx) = std::sync::mpsc::channel();
            let worker = std::thread::spawn(move || {
                result_tx
                    .send(worker_adapter.call(&worker_name, json!({"name":"n"})))
                    .unwrap();
            });
            requested_rx.recv_timeout(Duration::from_secs(1)).unwrap();
            assert_eq!(writes.load(Ordering::SeqCst), 0);
            assert!(matches!(
                result_rx.try_recv(),
                Err(std::sync::mpsc::TryRecvError::Empty)
            ));
            decision_tx.send(decision.clone()).unwrap();
            let result = result_rx.recv_timeout(Duration::from_secs(1)).unwrap();
            match decision {
                Ok(()) => assert_eq!(
                    result.unwrap()["structuredContent"]["receiptId"],
                    "confirmed"
                ),
                Err(error) => assert_eq!(result.unwrap_err(), error),
            }
            worker.join().unwrap();
        }
        assert_eq!(writes.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn current_scope_guards_cached_definitions_and_write_uses_host_handler() {
        let allowed = Arc::new(AtomicBool::new(true));
        let calls = Arc::new(AtomicUsize::new(0));
        let check = allowed.clone();
        let count = calls.clone();
        let mut adapter = ApplicationToolAdapter::new_local(
            Some(Arc::new(move |tool, args| {
                assert_eq!(tool.tool_name, "create");
                assert_eq!(args["name"], "n");
                count.fetch_add(1, Ordering::SeqCst);
                Ok(json!({"receiptId":"confirmed"}))
            })),
            Arc::new(move || {
                if check.load(Ordering::SeqCst) {
                    Ok(())
                } else {
                    Err("tool_scope_denied".into())
                }
            }),
        );
        adapter.add_write(tool()).unwrap();
        let name = adapter.list().unwrap()["tools"][0]["name"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_eq!(
            adapter.call(&name, json!({"name":"n"})).unwrap()["structuredContent"]["receiptId"],
            "confirmed"
        );
        allowed.store(false, Ordering::SeqCst);
        assert!(adapter.call(&name, json!({"name":"n"})).is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}

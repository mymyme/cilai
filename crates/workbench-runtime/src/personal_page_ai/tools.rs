use super::{
    ApplicationActionCallback, Arc, BridgeWriter, DocumentHandle, PageCallbacksFactory,
    PersonalPageAiManager, PersonalPageAuditMethod, PersonalPageBindingSnapshot,
    PersonalPageRiskLevel, PersonalPageTool, PersonalToolsCallback, ScopeValidator, Value, json,
    lock,
};
use crate::codex_harness::ApplicationWriteToolScope;

impl PersonalPageAiManager {
    pub(super) fn callbacks(
        &self,
        document: &str,
        doc: &DocumentHandle,
        ui: ApplicationActionCallback,
    ) -> PageCallbacksFactory {
        let manager = self.clone();
        let handle = Arc::clone(doc);
        let document = document.to_owned();
        Arc::new(move |run, validate| {
            let tools_granted = {
                let scope = lock(&handle);
                scope.binding.runtime_binding.is_some()
                    && (!scope.binding.page_ai.read_tools.is_empty()
                        || !scope.binding.page_ai.write_tools.is_empty())
            };
            let tools = if tools_granted {
                let manager = manager.clone();
                let handle = Arc::clone(&handle);
                let run = run.to_owned();
                let document = document.clone();
                let validate = Arc::clone(&validate);
                Some(Arc::new(move |request| {
                    manager.tool_request(&document, &handle, &run, &validate, &request)
                }) as PersonalToolsCallback)
            } else {
                None
            };
            let manager = manager.clone();
            let handle = Arc::clone(&handle);
            let run = run.to_owned();
            let document = document.clone();
            let ui = Arc::clone(&ui);
            let action: ApplicationActionCallback = Arc::new(move |name, arguments| {
                validate()?;
                let declaration = {
                    let scope = lock(&handle);
                    if !scope.actions.contains(name) {
                        return Err("personal_page_action_not_registered".into());
                    }
                    scope
                        .binding
                        .page_ai
                        .ui_actions
                        .iter()
                        .find(|action| action.action_id == name)
                        .cloned()
                        .ok_or("personal_page_action_not_declared")?
                };
                crate::app_bridge::validate_tool_arguments(&declaration.input_schema, &arguments)?;
                let tool = tool_scope(
                    &lock(&handle).binding,
                    name,
                    &declaration.title,
                    &declaration.description,
                    &declaration.input_schema,
                    &declaration.risk_level,
                );
                let execute = |args: &Value| {
                    manager.execute_page_action(
                        &document,
                        &handle,
                        &run,
                        (name, args),
                        &validate,
                        &ui,
                    )
                };
                manager
                    .ai
                    .confirm_personal_page_operation(&run, &tool, &arguments, &validate, &execute)
            });
            (tools, action)
        })
    }

    fn tool_request(
        &self,
        document: &str,
        doc: &DocumentHandle,
        run: &str,
        validate: &ScopeValidator,
        request: &Value,
    ) -> Result<Value, String> {
        validate()?;
        let action = request["action"]
            .as_str()
            .ok_or("personal_page_tool_request_invalid")?;
        let binding = lock(doc).binding.clone();
        let scoped_list = action == "list"
            && request["creationProjectId"].is_null()
            && request["packageDigest"].is_null();
        if !scoped_list
            && (request["creationProjectId"].as_str() != Some(binding.creation_project_id.as_str())
                || request["packageDigest"].as_str() != Some(binding.package_digest.as_str()))
        {
            return Err("personal_page_tool_scope_denied".into());
        }
        let declarations = binding
            .page_ai
            .read_tools
            .iter()
            .chain(&binding.page_ai.write_tools);
        let runtime = binding
            .runtime_binding
            .as_ref()
            .ok_or("personal_page_tools_not_granted")?;
        let runtime = serde_json::to_value(runtime).map_err(|_| "personal_page_binding_invalid")?;
        let catalog = crate::personal_runtime::invoke_binding(
            &self.ai.app_server()?,
            &runtime,
            None,
            &json!({}),
        )?;
        self.validate(doc, None)?;
        if action == "list" {
            let tools=declarations.map(|tool|{
                check_catalog(&catalog,tool)?;
                Ok(json!({"name":tool.name,"description":tool.description,"inputSchema":tool.input_schema}))
            }).collect::<Result<Vec<_>,String>>()?;
            return Ok(
                json!({"creationProjectId":binding.creation_project_id,"packageDigest":binding.package_digest,"tools":tools}),
            );
        }
        if action != "call" {
            return Err("personal_page_tool_request_invalid".into());
        }
        let name = request["tool"]
            .as_str()
            .ok_or("personal_page_tool_request_invalid")?;
        let declaration = declarations
            .into_iter()
            .find(|tool| tool.name == name)
            .cloned()
            .ok_or("personal_page_tool_not_declared")?;
        check_catalog(&catalog, &declaration)?;
        crate::app_bridge::validate_tool_arguments(
            &declaration.input_schema,
            &request["arguments"],
        )?;
        let tool = tool_scope(
            &binding,
            name,
            &declaration.title,
            &declaration.description,
            &declaration.input_schema,
            &PersonalPageRiskLevel::Medium,
        );
        let execute =
            |args: &Value| self.execute_page_tool(document, doc, run, name, args, validate);
        self.ai.confirm_personal_page_operation(
            run,
            &tool,
            &request["arguments"],
            validate,
            &execute,
        )
    }

    fn execute_page_action(
        &self,
        document: &str,
        doc: &DocumentHandle,
        run: &str,
        invocation: (&str, &Value),
        validate: &ScopeValidator,
        ui: &ApplicationActionCallback,
    ) -> Result<Value, String> {
        let (name, args) = invocation;
        validate()?;
        let _operation = self.ai.begin_capability_operation()?;
        let writer = BridgeWriter::prepare(self, document, lock(doc).revision)?;
        let result = (|| {
            let audit = self.audit_start(
                document,
                doc,
                &uuid::Uuid::new_v4().simple().to_string(),
                PersonalPageAuditMethod::UiActionInvoke,
                args,
                Some(run.into()),
            )?;
            let result = (|| {
                validate()?;
                ui(name, args.clone())
                    .map(|result| json!({"status":"succeeded","result":result}))
                    .map_err(|_| "personal_page_action_outcome_unconfirmed".to_owned())
            })();
            self.audit_finish(&audit, &result)?;
            if result.is_err() {
                lock(doc).retired = true;
            }
            result
        })();
        writer.complete(result)
    }

    fn execute_page_tool(
        &self,
        document: &str,
        doc: &DocumentHandle,
        run: &str,
        name: &str,
        args: &Value,
        validate: &ScopeValidator,
    ) -> Result<Value, String> {
        let binding = lock(doc).binding.clone();
        validate()?;
        let _operation = self.ai.begin_capability_operation()?;
        let writer = BridgeWriter::prepare(self, document, lock(doc).revision)?;
        let method = if binding
            .page_ai
            .read_tools
            .iter()
            .any(|tool| tool.name == name)
        {
            PersonalPageAuditMethod::ToolRead
        } else {
            PersonalPageAuditMethod::ToolWrite
        };
        let result = (|| {
            let audit = self.audit_start(
                document,
                doc,
                &uuid::Uuid::new_v4().simple().to_string(),
                method,
                args,
                Some(run.into()),
            )?;
            let result = (|| {
                let runtime = serde_json::to_value(
                    binding
                        .runtime_binding
                        .as_ref()
                        .ok_or("personal_page_tools_not_granted")?,
                )
                .map_err(|_| "personal_page_binding_invalid")?;
                validate()?;
                crate::personal_runtime::invoke_binding(&self.ai.app_server()?,&runtime,Some(name),args)
            .map(|output|json!({"status":if output["result"]["isError"]==true {"failed"}else{"succeeded"},"result":output["result"],"packageDigest":output["packageDigest"]}))
                .map_err(|_|"personal_page_tool_outcome_unconfirmed".to_owned())
            })();
            self.audit_finish(&audit, &result)?;
            if result.is_err()
                || result
                    .as_ref()
                    .is_ok_and(|value| value["status"] == "failed")
            {
                lock(doc).retired = true;
            }
            result
        })();
        writer.complete(result)
    }
}

fn check_catalog(catalog: &Value, tool: &PersonalPageTool) -> Result<(), String> {
    if !catalog["tools"].as_array().is_some_and(|tools| {
        tools
            .iter()
            .any(|item| item["name"] == tool.name && item["inputSchema"] == tool.input_schema)
    }) {
        return Err("personal_page_tool_catalog_changed".into());
    }
    Ok(())
}

fn tool_scope(
    binding: &PersonalPageBindingSnapshot,
    name: &str,
    title: &str,
    description: &str,
    schema: &Value,
    risk: &PersonalPageRiskLevel,
) -> ApplicationWriteToolScope {
    ApplicationWriteToolScope {
        application_id: binding.application_id.clone(),
        application_name: binding.name.clone(),
        dynamic_name: "personal_page".into(),
        tool_name: name.into(),
        title: title.into(),
        description: description.into(),
        input_schema: schema.clone(),
        risk_level: match risk {
            PersonalPageRiskLevel::Low => "low",
            PersonalPageRiskLevel::Medium => "medium",
            PersonalPageRiskLevel::High => "high",
        }
        .into(),
        timeout_ms: Some(30_000),
        confirmation_view: Some(json!({"reviewMode":"personalPageArguments","inputSchema":schema})),
    }
}

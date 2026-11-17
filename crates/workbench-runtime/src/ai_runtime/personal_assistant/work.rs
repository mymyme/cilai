use super::*;

pub(super) fn waiting(run: &AgentRun) -> Option<&crate::workbench_result::WorkInputRequest> {
    (run.status == "completed" && !run.superseded)
        .then_some(run.waiting_for_input.as_ref())
        .flatten()
}

pub(super) fn recorded_waiting<'a>(
    run: &'a AgentRun,
    assistant: &'a AssistantState,
) -> Option<(&'a str, &'a str)> {
    if let Some(input) = waiting(run) {
        return Some((&input.question, &input.impact));
    }
    assistant
        .waiting
        .get(&run.run_id)
        .filter(|input| run.status == "completed" && input.conversation_id == run.conversation_id)
        .map(|input| (input.question.as_str(), input.impact.as_str()))
}

pub(super) fn stopped(assistant: &AssistantState, run: &AgentRun) -> bool {
    let follows = || {
        assistant.followups.values().filter(|follow| {
            follow.conversation_id.as_deref() == Some(run.conversation_id.as_str())
        })
    };
    !follows().any(|follow| !follow.stopped)
        && follows().any(|follow| {
            follow.stopped
                && (follow.run_id.as_deref() == Some(run.run_id.as_str())
                    || follow.last_observation.as_deref() == Some(run.run_id.as_str()))
        })
}

pub(super) fn items(state: &ManagerState, assistant: &AssistantState) -> Vec<Value> {
    let mut latest = HashMap::new();
    for run in state.persisted.runs.iter().filter(|run| !run.superseded) {
        latest.insert(run.conversation_id.as_str(), run);
    }
    let mut items: Vec<_> = state.persisted.conversations.iter().filter_map(|conversation| {
        if conversation.archived || is_conversation(conversation) {
            return None;
        }
        let run = *latest.get(conversation.conversation_id.as_str())?;
        if stopped(assistant, run) { return None; }
        let task = conversation.work_classification.as_ref()
            .filter(|item| item.run_id == run.run_id)
            .and_then(|item| item.task_id.as_deref());
        let input = recorded_waiting(run, assistant);
        let status = if input.is_some() {
            "waiting_user"
        } else if run_is_active(&run.status) && !run.planning {
            run.status.as_str()
        } else {
            return None;
        };
        Some(json!({
            "id":conversation.conversation_id,"runId":run.run_id,"title":conversation.title,
            "projectId":conversation.context.project_id,"projectName":conversation.context.project_name,
            "taskId":task,"status":status,"waiting":input.map(|(question, impact)| json!({"question":question,"impact":impact})),
            "updatedAt":run.completed_at.unwrap_or(run.started_at)
        }))
    }).collect();
    items.sort_by_key(|item| std::cmp::Reverse(item["updatedAt"].as_i64()));
    items
}

#[derive(Clone, Copy)]
pub(super) enum TrackingView {
    Current,
    Completed,
    History,
}

impl TrackingView {
    fn includes(self, status: &str) -> bool {
        let current = matches!(
            status,
            "waiting_user"
                | "waiting_permission"
                | "failed"
                | "delivery_error"
                | "unavailable"
                | "interrupted"
                | "paused"
                | "pending"
                | "queued"
                | "running"
                | "terminating"
                | "permission_approved"
        );
        match self {
            Self::Current => current,
            Self::Completed => matches!(status, "completed" | "result_available"),
            Self::History => !current,
        }
    }
}

pub(super) fn tracking(
    state: &ManagerState,
    assistant: &AssistantState,
    work: &[Value],
    follows: &[Value],
    view: TrackingView,
    before: Option<&(i64, String)>,
) -> (Vec<Value>, Option<(i64, String)>) {
    const PAGE_SIZE: usize = 10;
    let mut reports = HashMap::new();
    let mut files = HashMap::new();
    let mut rows = BTreeMap::new();
    for report in &assistant.reports {
        let Ok(target) = visible_target(state, &report.conversation_id) else {
            continue;
        };
        if target.archived {
            continue;
        }
        let id = report.conversation_id.as_str();
        let latest = latest_run(state, id);
        reports.insert(id, report);
        if !report.artifacts.is_empty() {
            files.insert(id, report);
        }
        rows.insert(id.to_owned(), json!({
            "id":id,"runId":latest.map_or(&report.run_id, |run| &run.run_id),"title":target.title,
            "status":latest.map_or(report.status.as_str(), |run| observed_status(run, &assistant.waiting)),
            "updatedAt":latest.map_or(report.created_at, |run| run.completed_at.unwrap_or(run.started_at))
        }));
    }
    for follow in follows {
        let Some(operation) = follow["operationId"].as_str() else {
            continue;
        };
        let id = follow["conversationId"].as_str().unwrap_or(operation);
        let mut row = follow.clone();
        row["id"] = json!(id);
        rows.insert(id.to_owned(), row);
    }
    for item in work {
        let Some(id) = item["id"].as_str() else {
            continue;
        };
        let mut row = item.clone();
        if let Some(follow) = rows.get(id) {
            row["operationId"] = follow["operationId"].clone();
        }
        rows.insert(id.to_owned(), row);
    }
    let mut rows: Vec<_> = rows
        .into_iter()
        .filter(|(id, row)| {
            !latest_run(state, id).is_some_and(|run| stopped(assistant, run))
                && view.includes(row["status"].as_str().unwrap_or_default())
                && before.is_none_or(|(time, cursor_id)| {
                    (row["updatedAt"].as_i64().unwrap_or_default(), id.as_str())
                        < (*time, cursor_id.as_str())
                })
        })
        .collect();
    rows.sort_by(|(left_id, left), (right_id, right)| {
        (right["updatedAt"].as_i64(), right_id).cmp(&(left["updatedAt"].as_i64(), left_id))
    });
    let next = if !matches!(view, TrackingView::Current) && rows.len() > PAGE_SIZE {
        rows.truncate(PAGE_SIZE);
        rows.last()
            .and_then(|(id, row)| Some((row["updatedAt"].as_i64()?, id.clone())))
    } else {
        None
    };
    if rows.is_empty() {
        return (Vec::new(), next);
    }

    enrich_tracking_rows(state, assistant, &mut rows, &reports, &files);
    (rows.into_iter().map(|(_, row)| row).collect(), next)
}

fn enrich_tracking_rows(
    state: &ManagerState,
    assistant: &AssistantState,
    rows: &mut [(String, Value)],
    reports: &HashMap<&str, &Report>,
    files: &HashMap<&str, &Report>,
) {
    let mut anchors = HashMap::new();
    let mut operations = HashMap::new();
    let mut notice_order: Vec<_> = assistant
        .notices
        .values()
        .filter_map(|notice| {
            notice
                .publication
                .as_ref()
                .filter(|publication| publication.generated)
                .map(|publication| (publication.at, &notice.id, notice))
        })
        .collect();
    notice_order.sort_by_key(|(at, id, _)| (*at, *id));
    for (_, id, notice) in notice_order {
        for report in &notice.report_ids {
            anchors.insert(report.as_str(), id.as_str());
        }
        for result in &notice.results {
            operations.insert(
                (
                    result.operation_id.as_str(),
                    result.run_id.as_deref(),
                    result.status.as_str(),
                ),
                id.as_str(),
            );
        }
    }
    let user_messages: HashSet<_> = state
        .persisted
        .messages
        .iter()
        .filter(|message| {
            message.conversation_id == THREAD
                && message.role == "user"
                && assistant.notice_for_input(&message.message_id).is_none()
        })
        .map(|message| message.message_id.as_str())
        .collect();
    for (id, row) in rows {
        if let Ok(target) = visible_target(state, id) {
            row["projectName"] = json!(target.context.project_name);
            row["title"] = json!(target.title);
        }
        if let Some(report) = files.get(id.as_str()) {
            row["artifactRunId"] = json!(report.run_id);
            row["artifacts"] = json!(report.artifacts);
        }
        let run_id = row["runId"].as_str();
        let status = row["status"].as_str().unwrap_or_default();
        let operation = row["operationId"].as_str();
        let reminder = run_id.map(|run| format!("pa-work-input-{run}"));
        let message = reminder
            .as_deref()
            .filter(|id| assistant.work_messages.contains_key(*id))
            .or_else(|| {
                reports
                    .get(id.as_str())
                    .filter(|report| {
                        Some(report.run_id.as_str()) == run_id && report.status == status
                    })
                    .and_then(|report| anchors.get(report.id.as_str()).copied())
            })
            .or_else(|| operation.and_then(|op| operations.get(&(op, run_id, status)).copied()));
        if message.is_none() {
            let supplement = run_id
                .and_then(|id| state.persisted.runs.iter().find(|run| run.run_id == id))
                .and_then(|run| supplement_message(state, run));
            if let Some(supplement) = supplement {
                row["supplementMessageId"] = json!(supplement.message_id);
            } else if let Some(source) = operation
                .and_then(|op| assistant.followups.get(op))
                .filter(|follow| user_messages.contains(follow.source_message_id.as_str()))
            {
                row["requestMessageId"] = json!(source.source_message_id);
            }
        }
        row["messageId"] = json!(message);
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct WorkMessage {
    text: String,
    at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    run_id: Option<String>,
}

fn alert(assistant: &mut AssistantState, id: String, message: WorkMessage) -> bool {
    use std::collections::btree_map::Entry;
    match assistant.work_messages.entry(id) {
        Entry::Occupied(_) => false,
        Entry::Vacant(entry) => {
            entry.insert(message);
            true
        }
    }
}

pub(super) fn refresh(state: &ManagerState, assistant: &mut AssistantState) -> bool {
    let mut changed = false;
    for item in items(state, assistant) {
        let (Some(run_id), Some(question), Some(impact), Some(at)) = (
            item["runId"].as_str(),
            item["waiting"]["question"].as_str(),
            item["waiting"]["impact"].as_str(),
            item["updatedAt"].as_i64(),
        ) else {
            continue;
        };
        if assistant.waiting.contains_key(run_id) {
            continue;
        }
        changed |= alert(
            assistant,
            format!("pa-work-input-{run_id}"),
            WorkMessage {
                text: format!(
                    "{}\n\n{}",
                    summary::display_text(question, 500),
                    summary::display_text(impact, 500)
                ),
                at,
                run_id: Some(run_id.to_owned()),
            },
        );
    }
    for run in state.persisted.runs.iter().filter(|run| {
        run.response_to_work_run_id.is_some()
            && run.input_message_id.starts_with("pa-work-")
            && matches!(
                run.status.as_str(),
                "completed" | "failed" | "interrupted" | "terminated"
            )
            && waiting(run).is_none()
            && !run.superseded
    }) {
        let Some(target) = visible_target(state, &run.conversation_id)
            .ok()
            .filter(|target| !target.archived)
        else {
            continue;
        };
        let id = format!("pa-work-result-{}", run.run_id);
        if assistant.notices.contains_key(&id) {
            continue;
        }
        let report = report_id(run);
        let at = run.completed_at.unwrap_or(run.started_at);
        if !assistant.reports.iter().any(|item| item.id == report) {
            assistant.reports.push(Report {
                id: report.clone(),
                conversation_id: run.conversation_id.clone(),
                run_id: run.run_id.clone(),
                title: target.title.clone(),
                status: observed_status(run, &assistant.waiting).to_owned(),
                summary: preview(
                    run.output
                        .as_deref()
                        .or(run.error_reason.as_deref())
                        .unwrap_or(""),
                    1500,
                ),
                artifacts: run.artifacts.iter().take(12).cloned().collect(),
                created_at: at,
            });
        }
        assistant.notices.insert(
            id.clone(),
            SummaryNotice {
                id,
                source_run_id: run.run_id.clone(),
                report_ids: vec![report],
                results: vec![],
                created_at: at,
                state: "pending".into(),
                error: None,
                session: None,
                publication: None,
                rewrite: false,
            },
        );
        changed = true;
    }
    changed
}

fn supplement_message<'a>(
    state: &'a ManagerState,
    run: &AgentRun,
) -> Option<&'a ConversationMessage> {
    if run.response_to_work_run_id.is_none() || !run.input_message_id.starts_with("pa-work-") {
        return None;
    }
    state.persisted.messages.iter().find(|message| {
        message.message_id == run.input_message_id
            && message.conversation_id == run.conversation_id
            && message.role == "user"
    })
}

pub(super) fn messages(
    state: &ManagerState,
    assistant: &AssistantState,
) -> Vec<(i64, String, Value)> {
    let active: HashSet<_> = items(state, assistant)
        .into_iter()
        .filter(|item| item["waiting"].is_object())
        .filter_map(|item| item["runId"].as_str().map(str::to_owned))
        .collect();
    let mut messages: Vec<_> = assistant.work_messages.iter().filter(|(_, message)| message.run_id.is_some()).map(|(id, message)| {
        let active_run = message.run_id.as_deref().filter(|id| active.contains(*id));
        (message.at, id.clone(), json!({
            "messageId":id,"conversationId":THREAD,"role":"assistant","content":message.text,
            "createdAt":message.at,"attachments":[],"workRunId":active_run
        }))
    }).collect();
    for message in state
        .persisted
        .runs
        .iter()
        .filter_map(|run| supplement_message(state, run))
    {
        let id = &message.message_id;
        messages.push((
            message.created_at,
            id.clone(),
            json!({
                "messageId":id,"conversationId":THREAD,"role":"user",
                "content":super::super::personal_records::forwarded_user_content(&message.content),
                "createdAt":message.created_at,"attachments":[]
            }),
        ));
    }
    messages
}

pub(in crate::ai_runtime) fn expected_run(
    state: &ManagerState,
    conversation: &str,
    run_id: &str,
) -> Result<(), String> {
    if visible_target(state, conversation).is_ok_and(|target| !target.archived)
        && latest_run(state, conversation).is_some_and(|run| {
            run.run_id == run_id
                && (waiting(run).is_some()
                    || stored(state)
                        .is_ok_and(|assistant| recorded_waiting(run, &assistant).is_some()))
        })
    {
        Ok(())
    } else {
        Err("personal_assistant_work_input_stale".into())
    }
}

impl AiRuntimeManager {
    /// Route the user's explicit supplement through the original conversation submission.
    ///
    /// # Errors
    /// The work item changed, the identity changed, or the original execution rejected the message.
    pub fn supplement_personal_assistant_work(
        &self,
        session: &str,
        run_id: &str,
        message_id: &str,
        content: &str,
    ) -> Result<Value, String> {
        let content = content.trim();
        if content.is_empty()
            || content.chars().count() > 6000
            || message_id
                .strip_prefix("pa-work-")
                .is_none_or(|id| uuid::Uuid::parse_str(id).is_err())
        {
            return Err("personal_assistant_work_input_invalid".into());
        }
        let _operation = self.begin_capability_operation()?;
        let (command, mut args) = {
            let state = lock(&self.inner);
            check_session(&state, session)?;
            if let Some(accepted) = state
                .persisted
                .runs
                .iter()
                .find(|run| run.input_message_id == message_id)
            {
                let same = accepted.response_to_work_run_id.as_deref() == Some(run_id)
                    && state.persisted.messages.iter().any(|message| {
                        message.message_id == message_id
                            && message.conversation_id == accepted.conversation_id
                            && message.role == "user"
                            && super::super::personal_records::forwarded_user_content(
                                &message.content,
                            ) == content
                    });
                return if same {
                    Ok(json!({"runId":accepted.run_id,"conversationId":accepted.conversation_id}))
                } else {
                    Err("ai_message_identity_conflict".into())
                };
            }
            let run = state
                .persisted
                .runs
                .iter()
                .find(|run| run.run_id == run_id)
                .ok_or("personal_assistant_work_input_stale")?;
            expected_run(&state, &run.conversation_id, run_id)?;
            if run.thread_writer_active {
                return Err("personal_assistant_target_busy".into());
            }
            let target = visible_target(&state, &run.conversation_id)?;
            prepare_dispatch(
                &state,
                run,
                Some(target),
                Some(run),
                &DispatchInput {
                    operation: message_id,
                    title: &target.title,
                    message: content,
                    start: None,
                },
            )?
        };
        args["request"]["expectedWorkRunId"] = json!(run_id);
        let dispatch = lock(&self.assistant_dispatch)
            .clone()
            .ok_or("personal_assistant_dispatch_unavailable")?;
        dispatch(command, &args)
    }
}

#[derive(Default)]
pub(in crate::ai_runtime) struct BoardSources {
    session: String,
    projects: Option<Result<Vec<contexthub_protocol::MyWorkItem>, String>>,
}

impl BoardSources {
    fn project_result(&self) -> Result<&[contexthub_protocol::MyWorkItem], &str> {
        self.projects
            .as_ref()
            .ok_or("工作看板正在读取")
            .and_then(|result| result.as_deref().map_err(String::as_str))
    }

    pub(super) fn query(&self, session: &str, query: Option<&str>) -> Value {
        let query = query.unwrap_or_default().trim().to_lowercase();
        let matches = |texts: &[&str]| {
            texts
                .iter()
                .any(|text| text.to_lowercase().contains(&query))
        };
        if self.session != session {
            return json!({"error":"工作看板正在读取"});
        }
        let projects =
            match self.project_result() {
                Ok(items) => json!(items.iter()
                .filter(|item| matches(&[&item.title, &item.project_name]))
                .take(30).map(|item| json!({
                    "title":item.title, "project":item.project_name,"state":item.focus_group,
                    "taskId":item.task_id, "attentionId":item.attention_case_id,
                    "blockingReason":item.blocking_reason,
                })).collect::<Vec<_>>()),
                Err(error) => json!({"error":error}),
            };
        json!({"projectWork":projects})
    }
}

impl AiRuntimeManager {
    /// Refresh the authorized board sources outside the runtime lock.
    pub fn refresh_personal_assistant_board(&self) {
        let Some(dispatch) = lock(&self.assistant_dispatch).clone() else {
            return;
        };
        let session = {
            let mut state = lock(&self.inner);
            if state.shutting_down || state.data_dir.is_none() || state.session.id().is_empty() {
                return;
            }
            if state
                .assistant_board_refresh
                .is_some_and(|(started, active)| active || started.elapsed().as_secs() < 8)
            {
                return;
            }
            state.assistant_board_refresh = Some((Instant::now(), true));
            state.session.id().to_owned()
        };
        let manager = self.clone();
        if let Err(error) = std::thread::Builder::new().name("assistant-board".into()).spawn(move || {
            let projects = dispatch("app_server_request", &json!({
                "method":"myWork/get","params":{"projectId":null,"focusGroup":null,"limit":200}
            })).and_then(|value| serde_json::from_value::<contexthub_protocol::MyWorkGetResult>(value)
                .map(|result| result.items).map_err(|_| "工作看板返回无效".into()));
            let mut state = lock(&manager.inner);
            state.assistant_board_refresh = Some((Instant::now(), false));
            if check_session(&state, &session).is_err() { return; }
            state.assistant_board = BoardSources { session, projects: Some(projects) };
        }) {
            lock(&self.inner).assistant_board_refresh = Some((Instant::now(), false));
            eprintln!("personal_assistant_board_refresh_failed:{error}");
        }
    }
}

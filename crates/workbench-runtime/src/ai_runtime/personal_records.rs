use super::{
    AiRuntimeManager, ManagerState, data_dir_for_state, ensure_persistence_ready, lock,
    persist_session_locked, persistence_outcome_unknown, safe_storage_id, unix_epoch_millis,
};
use crate::persistent_threads::digest;
use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use uuid::Uuid;

mod reminders;
mod schedule;
mod tools;

pub(crate) use tools::{definitions, is_record_tool};

const PREFIX: &str = "personalRecords.";
const TODO: &str = "personalRecords.todo.";
const MEMO: &str = "personalRecords.memo.";
const NOTICE: &str = "personalRecords.notice.";
const RECEIPT: &str = "personalRecords.receipt.";
const NOTES_HEADER: &str = "\n\n【当前工作注意点】\n以下是用户关联的最新备忘录，只用于本次工作。先前自动附加的备忘录仅作历史资料。权限和当前用户要求以本轮为准。\n";

pub(super) fn owns_key(key: &str) -> bool {
    key.starts_with(PREFIX)
}

#[derive(Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum TodoState {
    Open,
    Completed,
    Canceled,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TodoInput {
    title: String,
    details: String,
    due_at: Option<String>,
    reminder: Option<schedule::ScheduleInput>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Todo {
    id: String,
    title: String,
    details: String,
    due_at: Option<i64>,
    reminder: Option<schedule::Schedule>,
    state: TodoState,
    revision: u64,
    source_message_id: Option<String>,
    created_at: i64,
    updated_at: i64,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum MemoScope {
    Project { id: String },
    Conversation { id: String },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MemoInput {
    title: String,
    body: String,
    url: Option<String>,
    scope: Option<MemoScope>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Memo {
    id: String,
    title: String,
    body: String,
    url: Option<String>,
    scope: Option<MemoScope>,
    scope_label: Option<String>,
    archived: bool,
    revision: u64,
    source_message_id: Option<String>,
    created_at: i64,
    updated_at: i64,
}

#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Filter {
    #[default]
    Current,
    History,
    All,
}

#[derive(Deserialize)]
#[serde(
    tag = "action",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum TodoAction {
    List {
        query: Option<String>,
        filter: Option<Filter>,
        offset: Option<usize>,
    },
    Get {
        id: String,
    },
    Save {
        id: Option<String>,
        revision: Option<u64>,
        item: TodoInput,
    },
    Complete {
        id: String,
        revision: u64,
    },
    Cancel {
        id: String,
        revision: u64,
    },
    Snooze {
        id: String,
        revision: u64,
        minutes: u32,
    },
}

#[derive(Deserialize)]
#[serde(
    tag = "action",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum MemoAction {
    List {
        query: Option<String>,
        filter: Option<Filter>,
        offset: Option<usize>,
    },
    Get {
        id: String,
    },
    Save {
        id: Option<String>,
        revision: Option<u64>,
        item: MemoInput,
    },
    Archive {
        id: String,
        revision: u64,
        archived: bool,
    },
    Delete {
        id: String,
        revision: u64,
    },
}

enum Action {
    Todo(TodoAction),
    Memo(MemoAction),
}

type Change = (String, Option<Value>);

fn decode<T: DeserializeOwned>(value: Value) -> Result<T, String> {
    serde_json::from_value(value).map_err(|_| "pa_record_input_invalid".into())
}

fn stored<T: DeserializeOwned>(value: &Value) -> Result<T, String> {
    serde_json::from_value(value.clone()).map_err(|_| "pa_record_storage_invalid".into())
}

fn encode<T: Serialize>(value: T) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|_| "pa_record_storage_invalid".into())
}

fn key(prefix: &str, id: &str) -> Result<String, String> {
    if id.len() > 160 || !safe_storage_id(id) {
        return Err("pa_record_input_invalid".into());
    }
    Ok(format!("{prefix}{id}"))
}

fn records<'a>(state: &'a ManagerState, prefix: &'a str) -> impl Iterator<Item = &'a Value> {
    state
        .persisted
        .conversation_session
        .range(prefix.to_owned()..)
        .take_while(move |(name, _)| name.starts_with(prefix))
        .map(|(_, value)| value)
}

fn get<T: DeserializeOwned>(state: &ManagerState, prefix: &str, id: &str) -> Result<T, String> {
    let key = key(prefix, id)?;
    stored(
        state
            .persisted
            .conversation_session
            .get(&key)
            .ok_or("pa_record_missing")?,
    )
}

fn change<T: Serialize>(prefix: &str, id: &str, data: T) -> Result<Change, String> {
    Ok((key(prefix, id)?, Some(encode(data)?)))
}

fn commit(state: &mut ManagerState, changes: Vec<Change>) -> Result<(), String> {
    if changes.is_empty() {
        return Ok(());
    }
    let values = &mut state.persisted.conversation_session;
    let previous = changes
        .into_iter()
        .map(|(name, value)| {
            let old = if let Some(value) = value {
                values.insert(name.clone(), value)
            } else {
                values.remove(&name)
            };
            (name, old)
        })
        .collect::<Vec<_>>();
    if let Err(error) = persist_session_locked(state) {
        if !persistence_outcome_unknown(state) {
            for (name, value) in previous {
                if let Some(value) = value {
                    state.persisted.conversation_session.insert(name, value);
                } else {
                    state.persisted.conversation_session.remove(&name);
                }
            }
        }
        return Err(error);
    }
    Ok(())
}

fn ready(state: &mut ManagerState) -> Result<(), String> {
    data_dir_for_state(state)?;
    if state.shutting_down || state.session.id().is_empty() {
        return Err("ai_profile_changed".into());
    }
    if persistence_outcome_unknown(state) {
        persist_session_locked(state)?;
    }
    ensure_persistence_ready(state)
}

fn check_session(state: &mut ManagerState, session: &str) -> Result<(), String> {
    if session.is_empty() || session != state.session.id() {
        return Err("ai_profile_changed".into());
    }
    ready(state)
}

fn field(value: &str, limit: usize, required: bool) -> Result<String, String> {
    let text = value.trim();
    if (required && text.is_empty()) || text.chars().count() > limit || text.contains('\0') {
        return Err("pa_record_input_invalid".into());
    }
    Ok(text.to_owned())
}

fn revision(current: u64, expected: u64) -> Result<u64, String> {
    if current != expected {
        return Err("pa_record_conflict".into());
    }
    current
        .checked_add(1)
        .ok_or("pa_record_storage_invalid".into())
}

fn text_matches(query: &str, text: &[&str]) -> bool {
    text.iter()
        .any(|value| value.to_lowercase().contains(query))
}

fn query_text(query: Option<&str>) -> Result<String, String> {
    field(query.unwrap_or_default(), 200, false).map(|value| value.to_lowercase())
}

fn matches_filter(filter: Option<Filter>, history: bool) -> bool {
    match filter.unwrap_or_default() {
        Filter::Current => !history,
        Filter::History => history,
        Filter::All => true,
    }
}

fn page(items: Vec<Value>, offset: Option<usize>, limit: usize) -> Value {
    let offset = offset.unwrap_or(0);
    let end = offset.saturating_add(limit);
    let next = (end < items.len()).then_some(end);
    json!({"items":items.into_iter().skip(offset).take(limit).collect::<Vec<_>>(),"nextOffset":next,"clock":clock()})
}

fn clock() -> Value {
    let time_zone = iana_time_zone::get_timezone().ok();
    json!({"localTime": Local::now().to_rfc3339(), "timeZone": time_zone})
}

impl Action {
    fn parse(kind: &str, request: Value) -> Result<Self, String> {
        match kind {
            "todo" => decode(request).map(Self::Todo),
            "memo" => decode(request).map(Self::Memo),
            _ => Err("pa_record_input_invalid".into()),
        }
    }

    fn reads(&self) -> bool {
        matches!(
            self,
            Self::Todo(TodoAction::List { .. } | TodoAction::Get { .. })
                | Self::Memo(MemoAction::List { .. } | MemoAction::Get { .. })
        )
    }

    fn read(self, state: &ManagerState, model: bool) -> Result<Value, String> {
        let limit = if model { 10 } else { 40 };
        match self {
            Self::Todo(TodoAction::Get { id }) => encode(get::<Todo>(state, TODO, &id)?),
            Self::Memo(MemoAction::Get { id }) => encode(get::<Memo>(state, MEMO, &id)?),
            Self::Todo(TodoAction::List {
                query,
                filter,
                offset,
            }) => {
                let query = query_text(query.as_deref())?;
                let mut items = records(state, TODO)
                    .map(stored::<Todo>)
                    .collect::<Result<Vec<_>, _>>()?;
                items.retain(|item| {
                    matches_filter(filter, item.state != TodoState::Open)
                        && text_matches(&query, &[&item.title, &item.details])
                });
                items.sort_by(|a, b| {
                    a.state
                        .ne(&TodoState::Open)
                        .cmp(&b.state.ne(&TodoState::Open))
                        .then_with(|| todo_time(a).cmp(&todo_time(b)))
                        .then_with(|| b.updated_at.cmp(&a.updated_at))
                        .then_with(|| a.id.cmp(&b.id))
                });
                let items = items
                    .into_iter()
                    .map(|mut item| {
                        if model {
                            item.details = item.details.chars().take(300).collect();
                        }
                        encode(item)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(page(items, offset, limit))
            }
            Self::Memo(MemoAction::List {
                query,
                filter,
                offset,
            }) => {
                let query = query_text(query.as_deref())?;
                let mut items = records(state, MEMO)
                    .map(stored::<Memo>)
                    .collect::<Result<Vec<_>, _>>()?;
                items.retain(|item| {
                    matches_filter(filter, item.archived)
                        && text_matches(
                            &query,
                            &[&item.title, &item.body, item.url.as_deref().unwrap_or("")],
                        )
                });
                items.sort_by(|a, b| {
                    b.updated_at
                        .cmp(&a.updated_at)
                        .then_with(|| a.id.cmp(&b.id))
                });
                let items = items
                    .into_iter()
                    .map(|mut item| {
                        if model {
                            item.body = item.body.chars().take(300).collect();
                        }
                        encode(item)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(page(items, offset, limit))
            }
            _ => Err("pa_record_input_invalid".into()),
        }
    }

    fn write(
        self,
        state: &ManagerState,
        now: i64,
        source: Option<String>,
    ) -> Result<(Change, Value), String> {
        match self {
            Self::Todo(action) => todo_write(action, state, now, source),
            Self::Memo(action) => memo_write(action, state, now, source),
        }
    }
}

fn todo_time(item: &Todo) -> i64 {
    item.due_at
        .or_else(|| {
            item.reminder
                .as_ref()
                .and_then(|schedule| schedule.snoozed_at.or(schedule.next_at))
        })
        .unwrap_or(i64::MAX)
}

fn todo_write(
    action: TodoAction,
    state: &ManagerState,
    now: i64,
    source: Option<String>,
) -> Result<(Change, Value), String> {
    let mut todo = match action {
        TodoAction::Save {
            id,
            revision: expected,
            item,
        } => {
            let mut todo = match (id, expected) {
                (None, None) => Todo {
                    id: Uuid::new_v4().to_string(),
                    title: String::new(),
                    details: String::new(),
                    due_at: None,
                    reminder: None,
                    state: TodoState::Open,
                    revision: 0,
                    source_message_id: source,
                    created_at: now,
                    updated_at: now,
                },
                (Some(id), Some(expected)) => active_todo(state, &id, expected)?,
                _ => return Err("pa_record_input_invalid".into()),
            };
            todo.title = field(&item.title, 160, true)?;
            todo.details = field(&item.details, 3000, false)?;
            todo.due_at = item
                .due_at
                .map(|at| {
                    DateTime::parse_from_rfc3339(&at)
                        .map(|time| time.timestamp_millis())
                        .map_err(|_| "pa_due_time_invalid")
                })
                .transpose()?;
            if item.reminder.as_ref() != todo.reminder.as_ref().map(|reminder| &reminder.input) {
                todo.reminder = item
                    .reminder
                    .map(|input| schedule::Schedule::new(input, todo.revision + 1, now))
                    .transpose()?;
            }
            todo
        }
        TodoAction::Complete {
            id,
            revision: expected,
        } => {
            let mut todo = active_todo(state, &id, expected)?;
            if let Some(reminder) = todo
                .reminder
                .as_mut()
                .filter(|plan| plan.input.repeat != schedule::Repeat::Once)
            {
                reminder.complete()?;
            } else {
                todo.state = TodoState::Completed;
            }
            todo
        }
        TodoAction::Cancel {
            id,
            revision: expected,
        } => {
            let mut todo = active_todo(state, &id, expected)?;
            todo.state = TodoState::Canceled;
            todo
        }
        TodoAction::Snooze {
            id,
            revision: expected,
            minutes,
        } => {
            let mut todo = active_todo(state, &id, expected)?;
            todo.reminder
                .as_mut()
                .ok_or("pa_reminder_missing")?
                .snooze(minutes, now)?;
            todo
        }
        _ => return Err("pa_record_input_invalid".into()),
    };
    if todo.state != TodoState::Open
        && let Some(reminder) = &mut todo.reminder
    {
        reminder.next_at = None;
        reminder.snoozed_at = None;
    }
    todo.revision = revision(todo.revision, todo.revision)?;
    todo.updated_at = now;
    let result = json!({"id":todo.id,"title":todo.title,"state":todo.state,"revision":todo.revision,
        "reminder":todo.reminder,"dueAt":todo.due_at});
    Ok((change(TODO, &todo.id, &todo)?, result))
}

fn active_todo(state: &ManagerState, id: &str, expected: u64) -> Result<Todo, String> {
    let todo: Todo = get(state, TODO, id)?;
    revision(todo.revision, expected)?;
    if todo.state != TodoState::Open {
        return Err("pa_todo_closed".into());
    }
    Ok(todo)
}

fn memo_write(
    action: MemoAction,
    state: &ManagerState,
    now: i64,
    source: Option<String>,
) -> Result<(Change, Value), String> {
    let mut memo = match action {
        MemoAction::Save {
            id,
            revision: expected,
            item,
        } => {
            let mut memo = match (id, expected) {
                (None, None) => Memo {
                    id: Uuid::new_v4().to_string(),
                    title: String::new(),
                    body: String::new(),
                    url: None,
                    scope: None,
                    scope_label: None,
                    archived: false,
                    revision: 0,
                    source_message_id: source,
                    created_at: now,
                    updated_at: now,
                },
                (Some(id), Some(expected)) => checked_memo(state, &id, expected)?,
                _ => return Err("pa_record_input_invalid".into()),
            };
            memo.title = field(&item.title, 160, true)?;
            memo.body = field(&item.body, 6000, false)?;
            memo.url = item.url.as_deref().map(memo_url).transpose()?;
            if memo.body.is_empty() && memo.url.is_none() {
                return Err("pa_memo_content_required".into());
            }
            memo.scope_label = item
                .scope
                .as_ref()
                .map(|scope| scope_label(state, scope))
                .transpose()?;
            memo.scope = item.scope;
            memo
        }
        MemoAction::Archive {
            id,
            revision: expected,
            archived,
        } => {
            let mut memo = checked_memo(state, &id, expected)?;
            memo.archived = archived;
            memo
        }
        MemoAction::Delete {
            id,
            revision: expected,
        } => {
            checked_memo(state, &id, expected)?;
            return Ok(((key(MEMO, &id)?, None), json!({"id":id,"deleted":true})));
        }
        _ => return Err("pa_record_input_invalid".into()),
    };
    memo.revision = revision(memo.revision, memo.revision)?;
    memo.updated_at = now;
    let result = json!({"id":memo.id,"title":memo.title,"revision":memo.revision,"archived":memo.archived,
        "scopeLabel":memo.scope_label});
    Ok((change(MEMO, &memo.id, &memo)?, result))
}

fn checked_memo(state: &ManagerState, id: &str, expected: u64) -> Result<Memo, String> {
    let memo: Memo = get(state, MEMO, id)?;
    revision(memo.revision, expected)?;
    Ok(memo)
}

fn memo_url(input: &str) -> Result<String, String> {
    let input = field(input, 2048, true)?;
    let parsed = url::Url::parse(&input).map_err(|_| "pa_memo_url_invalid")?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err("pa_memo_url_invalid".into());
    }
    Ok(parsed.into())
}

fn scope_label(state: &ManagerState, scope: &MemoScope) -> Result<String, String> {
    match scope {
        MemoScope::Project { id } => project_label(state, id),
        MemoScope::Conversation { id } => {
            let target = super::personal_assistant::visible_target(state, id)?;
            Ok(target.title.clone())
        }
    }
}

pub(super) fn project_label(state: &ManagerState, id: &str) -> Result<String, String> {
    key("", id)?;
    let value = state
        .server
        .as_ref()
        .ok_or("ai_runtime_not_initialized")?
        .request_result("project/get", &json!({"projectId":id}))?;
    value["project"]["displayName"]
        .as_str()
        .map(str::to_owned)
        .ok_or("pa_scope_unavailable".into())
}

pub(super) fn work_notes(
    state: &ManagerState,
    project_id: Option<&str>,
    conversation_id: Option<&str>,
    user_content: &str,
) -> Result<String, String> {
    let mut notes = records(state, MEMO)
        .map(stored::<Memo>)
        .collect::<Result<Vec<_>, _>>()?;
    notes.retain(|note| {
        !note.archived
            && match &note.scope {
                Some(MemoScope::Project { id }) => project_id == Some(id.as_str()),
                Some(MemoScope::Conversation { id }) => conversation_id == Some(id.as_str()),
                None => false,
            }
    });
    if notes.is_empty() && conversation_id.is_none() {
        return Ok(String::new());
    }
    if let Some(id) = project_id.filter(|_| {
        notes
            .iter()
            .any(|note| matches!(note.scope, Some(MemoScope::Project { .. })))
    }) {
        project_label(state, id)?;
    }
    notes.sort_by(|a, b| {
        b.updated_at
            .cmp(&a.updated_at)
            .then_with(|| a.id.cmp(&b.id))
    });
    let entries = notes
        .iter()
        .map(|note| {
            json!({"id":note.id,"revision":note.revision,
        "title":note.title,"body":note.body,"url":note.url})
        })
        .collect::<Vec<_>>();
    let envelope = json!({"sourceDigest":digest(&json!(user_content)),"items":entries});
    Ok(format!("{NOTES_HEADER}{envelope}"))
}

pub(super) fn forwarded_user_content(message: &str) -> &str {
    let Some(content) = message.strip_prefix("【个人助理转达】\n") else {
        return message;
    };
    if let Some((user, notes)) = content.rsplit_once(NOTES_HEADER)
        && let Ok(envelope) = serde_json::from_str::<Value>(notes)
        && envelope["items"].is_array()
        && envelope["sourceDigest"] == digest(&json!(user))
    {
        return user;
    }
    content
}

fn run_action(
    state: &mut ManagerState,
    action: Action,
    operation: Option<&str>,
    payload: &Value,
    source: Option<String>,
    model: bool,
) -> Result<Value, String> {
    if action.reads() {
        return action.read(state, model);
    }
    let receipt_key = key(RECEIPT, operation.ok_or("pa_operation_required")?)?;
    let fingerprint = digest(payload);
    if let Some(receipt) = state.persisted.conversation_session.get(&receipt_key) {
        return if receipt["digest"] == fingerprint {
            Ok(receipt["result"].clone())
        } else {
            Err("pa_operation_conflict".into())
        };
    }
    let (record, result) = action.write(state, unix_epoch_millis()?, source)?;
    let receipt = (
        receipt_key,
        Some(json!({"digest":fingerprint,"result":result})),
    );
    commit(state, vec![record, receipt])?;
    Ok(result)
}

impl AiRuntimeManager {
    /// Read or change the current user's personal records.
    ///
    /// # Errors
    /// The runtime changed, the request is invalid, or the write could not be verified.
    pub fn personal_records(
        &self,
        session: &str,
        kind: &str,
        request: &Value,
        operation: Option<&str>,
    ) -> Result<Value, String> {
        let action = Action::parse(kind, request.clone())?;
        let mut state = lock(&self.inner);
        check_session(&mut state, session)?;
        run_action(
            &mut state,
            action,
            operation,
            &json!([kind, request]),
            None,
            false,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linked_notes_are_scoped_and_a_removed_note_supersedes_history() {
        let mut state = ManagerState::default();
        let key = key(MEMO, "memo").unwrap();
        state.persisted.conversation_session.insert(key.clone(), json!({
            "id":"memo","title":"报价","body":"运费单列","scope":{"kind":"conversation","id":"quotation"},
            "archived":false,"revision":2,"createdAt":1,"updatedAt":2
        }));
        let content = "继续报价";
        let other = work_notes(&state, None, Some("another-work"), content).unwrap();
        assert!(!other.contains("运费单列"));
        let notes = work_notes(&state, None, Some("quotation"), content).unwrap();
        assert!(notes.contains("运费单列"));
        assert_eq!(
            forwarded_user_content(&format!("【个人助理转达】\n{content}{notes}")),
            content
        );
        state.persisted.conversation_session.get_mut(&key).unwrap()["archived"] = json!(true);
        let notes = work_notes(&state, None, Some("quotation"), content).unwrap();
        assert!(notes.contains("\"items\":[]"));
        assert!(!notes.contains("运费单列"));
    }
}

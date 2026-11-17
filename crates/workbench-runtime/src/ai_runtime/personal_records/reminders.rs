use super::{
    AiRuntimeManager, Change, ManagerState, NOTICE, TODO, Todo, TodoState, change, check_session,
    commit, digest, encode, get, lock, page, ready, records, revision, stored, unix_epoch_millis,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Desktop {
    Pending,
    Claimed,
    Submitted,
    Failed,
    Unavailable,
    Disabled,
    Skipped,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Notice {
    id: String,
    todo_id: String,
    title: String,
    generation: u64,
    occurrence_at: i64,
    original_at: i64,
    scheduled_at: i64,
    recorded_at: i64,
    count: u32,
    missed: bool,
    desktop: Desktop,
}

fn collect(
    state: &ManagerState,
    now: i64,
    recovering: bool,
) -> Result<(Vec<Change>, Vec<Value>), String> {
    let mut changes = Vec::new();
    let mut desktop = Vec::new();
    // The timestamp check uses the persisted projection so idle ticks avoid deserializing the list.
    for item in records(state, TODO).filter(|item| {
        item["state"] == "open"
            && ["nextAt", "snoozedAt"]
                .into_iter()
                .any(|field| item["reminder"][field].as_i64().is_some_and(|at| at <= now))
    }) {
        let mut todo: Todo = stored(item)?;
        let plan = todo.reminder.as_mut().ok_or("pa_record_storage_invalid")?;
        for occurrence in plan.due(now)? {
            let id = digest(&json!([
                todo.id,
                plan.generation,
                occurrence.first_at,
                occurrence.dispatch_at
            ]));
            let missed = recovering
                || occurrence.count > 1
                || now.saturating_sub(occurrence.dispatch_at) > 15_000;
            let notice = Notice {
                id,
                todo_id: todo.id.clone(),
                title: todo.title.clone(),
                generation: plan.generation,
                occurrence_at: if occurrence.count > 1 {
                    occurrence.dispatch_at
                } else {
                    occurrence.first_at
                },
                original_at: occurrence.first_at,
                scheduled_at: occurrence.dispatch_at,
                recorded_at: now,
                count: occurrence.count,
                missed,
                desktop: if missed {
                    Desktop::Skipped
                } else {
                    Desktop::Pending
                },
            };
            if !missed {
                desktop.push(json!({"id":notice.id,"todoId":notice.todo_id}));
            }
            changes.push(change(NOTICE, &notice.id, &notice)?);
        }
        todo.revision = revision(todo.revision, todo.revision)?;
        todo.updated_at = now;
        changes.push(change(TODO, &todo.id, &todo)?);
    }
    Ok((changes, desktop))
}

impl AiRuntimeManager {
    /// Advance due reminders for the currently active profile and persist application notices.
    ///
    /// # Errors
    /// The profile is unavailable or the write could not be verified.
    pub fn personal_reminder_tick(
        &self,
        previous_session: Option<&str>,
        waking: bool,
    ) -> Result<Value, String> {
        let mut state = lock(&self.inner);
        ready(&mut state)?;
        let session = state.session.id().to_owned();
        let (changes, desktop) = collect(
            &state,
            unix_epoch_millis()?,
            waking || previous_session != Some(session.as_str()),
        )?;
        let has_updates = !changes.is_empty();
        commit(&mut state, changes)?;
        Ok(json!({"runtimeSessionId":session,"changed":has_updates,"desktop":desktop}))
    }

    /// Claim one current desktop notification before calling the operating system.
    ///
    /// # Errors
    /// The profile or the reminder record changed, or the claim could not be verified.
    pub fn personal_reminder_claim(&self, session: &str, id: &str) -> Result<bool, String> {
        let mut state = lock(&self.inner);
        check_session(&mut state, session)?;
        let mut notice: Notice = get(&state, NOTICE, id)?;
        if notice.desktop != Desktop::Pending {
            return Ok(false);
        }
        let todo: Todo = get(&state, TODO, &notice.todo_id)?;
        let allowed = todo.state == TodoState::Open
            && todo.reminder.as_ref().is_some_and(|plan| {
                plan.generation == notice.generation
                    && plan.last_at == Some(notice.occurrence_at)
                    && !plan.completed
                    && plan.snoozed_at.is_none()
            });
        notice.desktop = if allowed {
            Desktop::Claimed
        } else {
            Desktop::Skipped
        };
        commit(&mut state, vec![change(NOTICE, id, &notice)?])?;
        Ok(allowed)
    }

    /// Record the actual result of attempting to hand a reminder to the desktop shell.
    ///
    /// # Errors
    /// The profile changed, the result is invalid, or persistence failed.
    pub fn personal_reminder_result(
        &self,
        session: &str,
        id: &str,
        outcome: &str,
    ) -> Result<(), String> {
        let mut state = lock(&self.inner);
        check_session(&mut state, session)?;
        let mut notice: Notice = get(&state, NOTICE, id)?;
        let result = match outcome {
            "submitted" => Desktop::Submitted,
            "failed" => Desktop::Failed,
            "unavailable" => Desktop::Unavailable,
            "disabled" => Desktop::Disabled,
            _ => return Err("pa_record_input_invalid".into()),
        };
        if notice.desktop == Desktop::Claimed
            || (notice.desktop == Desktop::Submitted && result == Desktop::Failed)
        {
            notice.desktop = result;
            commit(&mut state, vec![change(NOTICE, id, &notice)?])?;
        }
        Ok(())
    }

    /// List application reminder records for the requesting profile.
    ///
    /// # Errors
    /// The profile changed or the persisted records are invalid.
    pub fn personal_reminder_history(
        &self,
        session: &str,
        offset: Option<usize>,
    ) -> Result<Value, String> {
        let mut state = lock(&self.inner);
        check_session(&mut state, session)?;
        let mut items = records(&state, NOTICE)
            .map(stored::<Notice>)
            .collect::<Result<Vec<_>, _>>()?;
        items.sort_by(|a, b| {
            b.recorded_at
                .cmp(&a.recorded_at)
                .then_with(|| b.scheduled_at.cmp(&a.scheduled_at))
                .then_with(|| a.id.cmp(&b.id))
        });
        Ok(page(
            items
                .into_iter()
                .map(encode)
                .collect::<Result<Vec<_>, _>>()?,
            offset,
            40,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{ManagerState, NOTICE, collect, records};
    use crate::ai_runtime::personal_records::{decode, todo_write};
    use chrono::DateTime;
    use serde_json::json;

    #[test]
    fn recovery_persists_one_missed_notice_and_advances_the_schedule() {
        let mut state = ManagerState::default();
        let at = DateTime::parse_from_rfc3339("2026-11-12T02:00:00Z")
            .unwrap()
            .timestamp_millis();
        let (record, _) = todo_write(decode(json!({"action":"save","item":{
            "title":"确认报价","details":"","reminder":{"localTime":"2026-11-12T10:00","timeZone":"Asia/Shanghai","repeat":"daily"}
        }})).unwrap(), &state, at - 60_000, None).unwrap();
        state
            .persisted
            .conversation_session
            .insert(record.0, record.1.unwrap());
        let now = at + 86_400_000;
        let (changes, desktop) = collect(&state, now, true).unwrap();
        assert!(desktop.is_empty());
        for (key, value) in changes {
            state
                .persisted
                .conversation_session
                .insert(key, value.unwrap());
        }
        let notices = records(&state, NOTICE).collect::<Vec<_>>();
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0]["count"], 2);
        assert_eq!(notices[0]["originalAt"], at);
        assert_eq!(notices[0]["missed"], true);
        assert!(collect(&state, now, false).unwrap().0.is_empty());
        assert_eq!(collect(&state, now + 86_400_000, false).unwrap().1.len(), 1);
    }
}

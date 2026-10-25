use super::{AgentRun, CodexHarnessEvent, MAX_LOG_LINES, ProgressPersistence};
use serde_json::json;

fn run() -> AgentRun {
    serde_json::from_value(json!({
        "runId":"run","conversationId":"conversation","agentId":"agent",
        "inputMessageId":"input","provider":"codex","status":"running","startedAt":1
    }))
    .unwrap()
}

fn event(item: &str, phase: &str, at: i64) -> CodexHarnessEvent {
    CodexHarnessEvent {
        event_type: "item.started".into(),
        item_id: Some(item.into()),
        phase: phase.into(),
        kind: "command".into(),
        status: "started".into(),
        summary: "开始".into(),
        duration_ms: None,
        detail_available: false,
        streamed_output: None,
        steered_input: None,
        context_usage: None,
        read_paths: Vec::new(),
        created_at: at,
    }
}

#[test]
fn progress_merges_items_by_phase_and_keeps_original_sequence_and_time() {
    let mut run = run();
    assert!(run.observe_progress(event("item", "reply", 10)) == ProgressPersistence::Required);
    let mut completed = event("item", "reply", 40);
    completed.status = "completed".into();
    completed.summary = "已完成".into();
    completed.detail_available = true;
    completed.read_paths = vec!["/project/a.md".into()];
    completed.context_usage = Some((123, Some(1000)));
    run.observe_progress(completed);
    run.observe_progress(event("item", "classification", 50));
    assert_eq!(run.events.len(), 2);
    assert_eq!(run.events[0].sequence, 1);
    assert_eq!(run.events[0].created_at, 10);
    assert_eq!(run.events[0].duration_ms, Some(30));
    assert_eq!(run.events[0].status, "completed");
    assert!(run.events[0].detail_available);
    assert_eq!(run.events[0].read_paths, ["/project/a.md"]);
    assert_eq!(run.events[1].sequence, 2);
    assert_eq!(run.usage.context.as_ref().unwrap().tokens, 123);
    assert_eq!(
        run.usage.context.as_ref().unwrap().window_tokens,
        Some(1000)
    );
}

#[test]
fn streamed_output_is_transient_and_steering_preserves_the_current_reply() {
    let mut run = run();
    let mut streamed = event("reply", "reply", 1);
    streamed.streamed_output = Some("答复".into());
    assert!(run.observe_progress(streamed.clone()) == ProgressPersistence::Transient);
    run.record_steered_input(event("steer", "reply", 2));
    assert_eq!(run.partial_output.as_deref(), Some("答复"));
    assert_eq!(run.events.len(), 1);
    assert!(run.log_tail.is_empty());
    streamed.streamed_output = Some(String::new());
    assert!(run.observe_progress(streamed) == ProgressPersistence::Transient);
    assert!(run.partial_output.is_none());
    assert_eq!(run.events.len(), 1);
}

#[test]
fn progress_limits_log_tail_without_discarding_persisted_event_history() {
    let mut run = run();
    for index in 0..(MAX_LOG_LINES + 10) {
        let mut progress = event(&format!("item-{index}"), "reply", 1);
        progress.summary = index.to_string();
        run.observe_progress(progress);
    }
    assert_eq!(run.log_tail.len(), MAX_LOG_LINES);
    assert_eq!(run.log_tail[0], "10");
    assert_eq!(run.events.len(), MAX_LOG_LINES + 10);
    assert_eq!(
        run.events.last().unwrap().sequence,
        u32::try_from(MAX_LOG_LINES + 10).unwrap()
    );
}

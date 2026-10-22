use super::history_items;
use super::imported_prefix;
use super::model_fragments;
use codex_app_server_protocol::ThreadHistoryImportParams;
use codex_protocol::ThreadId;
use pretty_assertions::assert_eq;
use serde_json::json;

fn fixture(thread_id: ThreadId) -> ThreadHistoryImportParams {
    serde_json::from_value(json!({
        "threadId":thread_id.to_string(),
        "turns":[{"id":"legacy-run-1","startedAt":10,"completedAt":11,"error":null,"displayNotes":"显示活动","modelContext":"只读附件引用",
        "messages":[{"id":"m1","role":"user","text":"旧问题\n第二行"},
                    {"id":"m2","role":"assistant","text":"旧回答"}]}]
    }))
    .unwrap()
}

#[test]
fn import_replay_and_partial_prefix_never_duplicate_messages() {
    let id = ThreadId::new();
    let params = fixture(id);
    let items = history_items(id, &params).unwrap();
    for count in 0..=items.len() {
        assert_eq!(imported_prefix(&items[..count], &items).unwrap(), count);
    }
    let mut changed = params;
    changed.turns[0].messages[0].text = "不同的原文".into();
    let other = history_items(id, &changed).unwrap();
    assert!(imported_prefix(&items, &other).is_err());
    let mut duplicated = items.clone();
    duplicated.extend(items.clone());
    assert!(imported_prefix(&duplicated, &items).is_err());
}

#[test]
fn import_rejects_duplicate_ids_and_preserves_timestamps_and_failure() {
    let id = ThreadId::new();
    let mut params = fixture(id);
    params.turns[0].error = Some("旧轮次已中断".into());
    let items = history_items(id, &params).unwrap();
    let last = serde_json::to_value(items.last().unwrap()).unwrap();
    assert_eq!(last["payload"]["error"]["message"], "旧轮次已中断");
    assert_eq!(last["payload"]["started_at"], 10);
    assert_eq!(last["payload"]["completed_at"], 11);
    params.turns[0].messages[1].id = "m1".into();
    assert!(history_items(id, &params).is_err());
}

#[test]
fn long_unicode_history_is_complete_and_model_items_are_bounded() {
    let text = "中文历史\n".repeat(12000);
    let fragments = model_fragments(&text);
    assert!(fragments.iter().all(|fragment| fragment.len() <= 8000));
    assert_eq!(fragments.concat(), text);
    assert_eq!(model_fragments(""), vec![""]);
}

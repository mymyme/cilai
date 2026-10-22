use super::{
    Arc, BTreeMap, HashMap, HashSet, Mutex, PersonalPageBindingParams, PersonalPageBindingSnapshot,
    Value, json,
};
use std::sync::atomic::AtomicBool;
use std::sync::{MutexGuard, TryLockError};
use std::time::{Duration, Instant};

pub(super) struct Registration {
    pub gate: Arc<Mutex<()>>,
    pub retired: Arc<AtomicBool>,
}

#[derive(Clone)]
pub(super) enum DocumentEntry {
    Registering(Arc<Registration>),
    Active(DocumentHandle),
    Retired(Value),
}

pub(super) fn no_scope_receipt() -> Value {
    json!({"retired":true,"processesExited":true,"cleanupPending":false,"auditPending":false,"notAdmitted":true})
}

pub(super) fn bounded_gate(
    gate: &Mutex<()>,
    timeout: Duration,
) -> Result<MutexGuard<'_, ()>, String> {
    let start = Instant::now();
    loop {
        match gate.try_lock() {
            Ok(guard) => return Ok(guard),
            Err(TryLockError::Poisoned(error)) => return Ok(error.into_inner()),
            Err(TryLockError::WouldBlock) => {}
        }
        if start.elapsed() >= timeout {
            return Err("personal_page_close_unconfirmed".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[derive(Clone)]
pub(super) struct Context {
    pub expires_at: i64,
    pub value: Value,
}

pub(super) struct Document {
    pub gate: Arc<Mutex<()>>,
    pub cancellation: Arc<AtomicBool>,
    pub params: PersonalPageBindingParams,
    pub binding: PersonalPageBindingSnapshot,
    pub session: String,
    pub revision: u64,
    pub retired: bool,
    pub registration_unconfirmed: bool,
    pub contexts: BTreeMap<String, Context>,
    pub actions: HashSet<String>,
    pub model_sessions: HashMap<String, u64>,
    pub runs: HashSet<String>,
    pub requests: HashMap<String, RequestReceipt>,
}

pub(super) struct RequestReceipt {
    pub digest: String,
    pub result: Option<Result<Value, String>>,
}

pub(super) type DocumentHandle = Arc<Mutex<Document>>;

pub(super) fn lock<T>(value: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    value
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub(super) fn id(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 160
        || !value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b':' | b'.'))
    {
        return Err("personal_page_identity_invalid".into());
    }
    Ok(())
}

pub(super) fn digest(value: &Value) -> String {
    use sha2::{Digest as _, Sha256};
    format!("sha256:{:x}", Sha256::digest(value.to_string().as_bytes()))
}

pub(super) fn now() -> Result<i64, String> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|time| i64::try_from(time.as_millis()).ok())
        .ok_or_else(|| "personal_page_clock_unavailable".into())
}

pub(super) fn parse<T: serde::de::DeserializeOwned>(value: &Value) -> Result<T, String> {
    serde_json::from_value(value.clone()).map_err(|_| "personal_page_request_invalid".into())
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Invocation {
    pub method: String,
    pub request_id: String,
    #[serde(default = "empty_object")]
    pub params: Value,
}

fn empty_object() -> Value {
    json!({})
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Publish {
    pub context_id: String,
    pub context_type: String,
    pub schema_version: String,
    pub title: String,
    pub summary: String,
    pub data: Value,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ContextId {
    pub context_id: String,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Actions {
    pub action_ids: Vec<String>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct SessionId {
    pub model_session_id: String,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct RunId {
    pub run_id: String,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct RunCreate {
    pub model_session_id: String,
    pub model_id: String,
    pub prompt: String,
    pub client_message_id: String,
}

#[cfg(test)]
mod tests {
    use super::{DocumentEntry, lock};
    use crate::ai_runtime::AiRuntimeManager;
    use crate::personal_page_ai::PersonalPageAiManager;
    use contexthub_protocol::PersonalPageBindingParams;
    use std::sync::atomic::Ordering;
    use std::time::{Duration, Instant};

    fn binding() -> PersonalPageBindingParams {
        PersonalPageBindingParams {
            creation_project_id: format!("capability_{}", "a".repeat(32)),
            expected_package_digest: format!("sha256:{}", "b".repeat(64)),
        }
    }

    #[test]
    fn admission_failure_keeps_a_nonce_tombstone_that_closes_without_runtime_resources() {
        let manager = PersonalPageAiManager::new(AiRuntimeManager::new());
        for index in 0..140 {
            let doc = format!("failed-{index}");
            assert!(manager.register_document(&doc, binding()).is_err());
            let closed = manager.close_document(&doc).unwrap();
            assert_eq!(closed["notAdmitted"], true);
            assert_eq!(closed["processesExited"], true);
            assert_eq!(closed["cleanupPending"], false);
            assert_eq!(manager.close_document(&doc).unwrap(), closed);
            assert_eq!(
                manager.register_document(&doc, binding()).unwrap_err(),
                "personal_page_document_reused"
            );
            assert!(matches!(
                lock(&manager.documents).get(&doc),
                Some(DocumentEntry::Retired(_))
            ));
        }
        assert_eq!(manager.revoke_all().unwrap()["processesExited"], true);
        assert_eq!(
            manager.close_document("never-registered").unwrap_err(),
            "personal_page_document_required"
        );
    }

    #[test]
    fn capacity_counts_unfinished_scopes_and_closed_tombstones_cannot_reopen() {
        let manager = PersonalPageAiManager::new(AiRuntimeManager::new());
        for index in 0..128 {
            manager
                .reserve_document(&format!("pending-{index}"))
                .unwrap();
        }
        assert_eq!(
            manager.reserve_document("overflow").err().unwrap(),
            "personal_page_document_limit"
        );
        assert_eq!(
            manager.close_document("overflow").unwrap()["notAdmitted"],
            true
        );
        manager.close_document("pending-0").unwrap();
        assert!(manager.reserve_document("new-active").is_ok());
        assert_eq!(
            manager.reserve_document("pending-0").err().unwrap(),
            "personal_page_document_reused"
        );
    }

    #[test]
    fn close_marks_pending_registration_before_waiting_and_cannot_report_early_exit() {
        let manager = PersonalPageAiManager::new(AiRuntimeManager::new());
        let registration = manager.reserve_document("pending").unwrap();
        let guard = lock(&registration.gate);
        let worker_manager = manager.clone();
        let (sender, receiver) = std::sync::mpsc::channel();
        let child = std::thread::spawn(move || {
            sender
                .send(worker_manager.close_document("pending"))
                .unwrap();
        });
        let start = Instant::now();
        while !registration.retired.load(Ordering::SeqCst)
            && start.elapsed() < Duration::from_secs(1)
        {
            std::thread::yield_now();
        }
        assert!(registration.retired.load(Ordering::SeqCst));
        assert!(receiver.try_recv().is_err());
        drop(guard);
        let closed = receiver
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .unwrap();
        child.join().unwrap();
        assert_eq!(closed["notAdmitted"], true);
        assert_eq!(
            manager.register_document("pending", binding()).unwrap_err(),
            "personal_page_document_reused"
        );
    }

    #[test]
    fn stalled_registration_close_is_bounded_and_retains_the_unconfirmed_scope() {
        let manager = PersonalPageAiManager::new(AiRuntimeManager::new());
        let registration = manager.reserve_document("stalled").unwrap();
        let guard = lock(&registration.gate);
        let started = Instant::now();
        assert_eq!(
            manager.close_document("stalled").unwrap_err(),
            "personal_page_close_unconfirmed"
        );
        assert!(started.elapsed() < Duration::from_secs(12));
        assert!(registration.retired.load(Ordering::SeqCst));
        assert!(matches!(
            lock(&manager.documents).get("stalled"),
            Some(DocumentEntry::Registering(_))
        ));
        drop(guard);
        assert_eq!(
            manager.close_document("stalled").unwrap()["notAdmitted"],
            true
        );
    }
}

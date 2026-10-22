use super::{AiRuntimeManager, Value, codex_threads, lock};

impl AiRuntimeManager {
    pub(crate) fn mark_personal_page_conversation_closed(
        &self,
        session: &str,
    ) -> Result<(), String> {
        Self::application_store(
            &self.app_server()?,
            "close",
            &serde_json::json!({"sessionId":session}),
        )?;
        Ok(())
    }

    pub(crate) fn cleanup_personal_page_conversation(&self, session: &str) -> Result<bool, String> {
        self.cleanup_application_conversation(session)
    }

    pub(super) fn with_application_native_scope<T>(
        &self,
        journal: &Value,
        action: impl FnOnce(&codex_threads::CodexThreadService) -> Result<T, String>,
    ) -> Result<T, String> {
        let default = lock(&self.inner)
            .application_threads
            .clone()
            .ok_or("application_thread_service_unavailable")?;
        let scope = &journal["binding"]["runtimeScope"];
        if scope.is_null() {
            return action(&default);
        }
        if scope["runtimeKind"] != "application" {
            return Err("application_thread_scope_invalid".into());
        }
        let service = default.for_runtime_scope(scope)?;
        let result = action(&service);
        if service.shutdown_checked().is_err() {
            lock(&self.inner).harness_retirement_error =
                Some("application_history_process_exit_unconfirmed".into());
            return Err("application_history_process_exit_unconfirmed".into());
        }
        result
    }
}

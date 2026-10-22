use super::{
    AiRuntimeManager, ApplicationModelWriteRequest, ApplicationWriteApproval,
    ApplicationWriteToolScope, Value, application_write_arguments,
    application_write_receipt_succeeded, lock, unix_epoch_millis,
};
use crate::application_tool_adapter::ScopeValidator;
use std::sync::mpsc;
use std::time::Duration;

impl AiRuntimeManager {
    pub(crate) fn await_personal_page_runs_exit(&self, runs: &[String]) -> Result<(), String> {
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        let mut state = lock(&self.inner);
        while runs.iter().any(|run| {
            state.harness_controls.contains_key(run)
                || state
                    .application_model_runs
                    .get(run)
                    .is_some_and(|r| r.pid.is_some())
        }) {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                return Err("personal_page_process_exit_unconfirmed".into());
            }
            let (next, _) = self
                .application_runs_completed
                .wait_timeout(state, remaining)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state = next;
        }
        Ok(())
    }

    pub(crate) fn confirm_personal_page_operation(
        &self,
        run: &str,
        tool: &ApplicationWriteToolScope,
        arguments: &Value,
        validate: &ScopeValidator,
        execute: &dyn Fn(&Value) -> Result<Value, String>,
    ) -> Result<Value, String> {
        validate()?;
        let (id, receiver) = self.prepare_personal_page_confirmation(run, tool, arguments)?;
        let approval = self.await_personal_page_confirmation(run, &id, &receiver, validate)?;
        validate()?;
        let arguments = application_write_arguments(tool, &approval.arguments)?;
        let result = execute(&arguments);
        self.finish_personal_page_confirmation(run, &id, &result)?;
        result
    }

    fn prepare_personal_page_confirmation(
        &self,
        run: &str,
        tool: &ApplicationWriteToolScope,
        arguments: &Value,
    ) -> Result<(String, mpsc::Receiver<Result<i64, String>>), String> {
        let id = uuid::Uuid::new_v4().to_string();
        let requested_at = unix_epoch_millis()?;
        let (sender, receiver) = mpsc::channel();
        let mut state = lock(&self.inner);
        let record = state
            .application_model_runs
            .get_mut(run)
            .ok_or("application_model_run_not_found")?;
        if record.cancel_requested
            || !matches!(record.snapshot.status.as_str(), "queued" | "running")
        {
            return Err("application_write_run_not_active".into());
        }
        record.snapshot.write_request = Some(ApplicationModelWriteRequest {
            approval: ApplicationWriteApproval {
                confirmation_id: id.clone(),
                application_id: tool.application_id.clone(),
                application_name: tool.application_name.clone(),
                tool_name: tool.tool_name.clone(),
                tool_title: tool.title.clone(),
                description: tool.description.clone(),
                risk_level: tool.risk_level.clone(),
                arguments: application_write_arguments(tool, arguments)?,
                confirmation_view: tool.confirmation_view.clone(),
                execution_status: Some("awaiting_confirmation".into()),
                receipt: None,
                failure_reason: None,
            },
            requested_at,
            valid_until: requested_at.saturating_add(300_000),
        });
        record.snapshot.status = "waiting_confirmation".into();
        state
            .application_model_write_controls
            .insert(id.clone(), sender);
        drop(state);
        if let Err(error) = self.persist_application_run(run) {
            lock(&self.inner)
                .application_model_write_controls
                .remove(&id);
            return Err(error);
        }
        Ok((id, receiver))
    }

    fn await_personal_page_confirmation(
        &self,
        run: &str,
        id: &str,
        receiver: &mpsc::Receiver<Result<i64, String>>,
        validate: &ScopeValidator,
    ) -> Result<ApplicationWriteApproval, String> {
        loop {
            if let Err(error) = validate() {
                self.finish_personal_page_confirmation(run, id, &Err(error.clone()))?;
                return Err(error);
            }
            match receiver.recv_timeout(Duration::from_millis(250)) {
                Ok(Ok(_)) => break,
                Ok(Err(error)) => return Err(error),
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err("application_write_confirmation_not_active".into());
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            let expired = lock(&self.inner)
                .application_model_runs
                .get(run)
                .and_then(|r| r.snapshot.write_request.as_ref())
                .is_none_or(|request| {
                    request.valid_until < unix_epoch_millis().unwrap_or(i64::MAX)
                });
            if expired {
                self.finish_personal_page_confirmation(
                    run,
                    id,
                    &Err("application_write_confirmation_expired".into()),
                )?;
                return Err("application_write_confirmation_expired".into());
            }
        }
        lock(&self.inner)
            .application_model_runs
            .get(run)
            .and_then(|r| r.snapshot.write_request.as_ref())
            .filter(|request| request.approval.confirmation_id == id)
            .map(|request| request.approval.clone())
            .ok_or_else(|| "application_write_confirmation_missing".into())
    }

    fn finish_personal_page_confirmation(
        &self,
        run: &str,
        id: &str,
        result: &Result<Value, String>,
    ) -> Result<(), String> {
        let mut state = lock(&self.inner);
        state.application_model_write_controls.remove(id);
        let record = state
            .application_model_runs
            .get_mut(run)
            .ok_or("application_model_run_not_found")?;
        let approval = &mut record
            .snapshot
            .write_request
            .as_mut()
            .filter(|request| request.approval.confirmation_id == id)
            .ok_or("application_write_confirmation_missing")?
            .approval;
        match result {
            Ok(receipt) => {
                approval.receipt = Some(receipt.clone());
                approval.execution_status = Some(
                    if application_write_receipt_succeeded(receipt) {
                        "succeeded"
                    } else {
                        "failed"
                    }
                    .into(),
                );
            }
            Err(error) => {
                approval.execution_status = Some(
                    if error.ends_with("_unconfirmed") || error.ends_with("_unknown") {
                        "outcome_unknown"
                    } else {
                        "failed"
                    }
                    .into(),
                );
                approval.failure_reason = Some(error.clone());
            }
        }
        if !record.cancel_requested {
            record.snapshot.status = "running".into();
        }
        drop(state);
        self.persist_application_run(run)
    }
}

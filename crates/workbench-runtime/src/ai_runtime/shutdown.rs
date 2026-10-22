use super::{AiRuntimeManager, Instant, lock, maintenance, unix_epoch_millis};

impl AiRuntimeManager {
    pub(super) fn shutdown_application_runs(&self) {
        let _lifecycle = lock(&self.application_lifecycle);
        let applications = {
            let mut state = lock(&self.inner);
            state.shutting_down = true;
            state
                .application_model_runs
                .iter()
                .filter(|(_, record)| {
                    matches!(
                        record.snapshot.status.as_str(),
                        "queued" | "running" | "waiting_confirmation" | "submitting"
                    )
                })
                .map(|(id, _)| id.clone())
                .collect::<Vec<_>>()
        };
        for run_id in applications {
            if let Err(error) = self.cancel_application_model_run(&run_id) {
                eprintln!("application_shutdown_failed:{error}");
            }
        }
        let mut state = lock(&self.inner);
        let deadline = Instant::now() + std::time::Duration::from_secs(20);
        while state.application_model_runs.iter().any(|(id, record)| {
            record.snapshot.status == "terminating" || state.harness_controls.contains_key(id)
        }) {
            if Instant::now() >= deadline {
                state.harness_retirement_error = Some("application_shutdown_unconfirmed".into());
                break;
            }
            state = self
                .application_runs_completed
                .wait_timeout(state, std::time::Duration::from_millis(100))
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .0;
        }
        let application_threads = state.application_threads.take();
        drop(state);
        if let Some(service) = application_threads
            && let Err(error) = service.shutdown_checked()
        {
            lock(&self.inner).harness_retirement_error = Some(error);
        }
    }

    pub(super) fn stop_owned_harness_controls(&self) -> bool {
        let (controls, changed, tickets) = {
            let mut state = lock(&self.inner);
            let submitted = state
                .submitted_harnesses
                .keys()
                .cloned()
                .collect::<Vec<_>>();
            let tickets = state
                .submitted_harnesses
                .values()
                .filter_map(|(_, _, input)| input.automation_ticket.clone())
                .collect::<Vec<_>>();
            state.submitted_harnesses.clear();
            for run in &mut state.persisted.runs {
                if submitted.contains(&run.run_id) {
                    run.status = "interrupted".into();
                    run.completed_at = unix_epoch_millis().ok();
                    run.error_reason = Some("工作台已关闭，运行未开始。".into());
                }
            }
            let confirmations = state.application_write_controls.drain().collect::<Vec<_>>();
            let changed = !submitted.is_empty() || !confirmations.is_empty();
            for (id, sender) in confirmations {
                if let Some(request) =
                    state
                        .persisted
                        .permission_requests
                        .iter_mut()
                        .find(|request| {
                            request.permission_request_id == id && request.status == "pending"
                        })
                {
                    request.status = "cancelled".into();
                    request.decided_at = unix_epoch_millis().ok();
                    if let Some(approval) = request.application_write.as_mut() {
                        approval.execution_status = Some(
                            if approval.execution_status.as_deref() == Some("submitting") {
                                "outcome_unknown"
                            } else {
                                "cancelled"
                            }
                            .into(),
                        );
                    }
                }
                let _ = sender.send(Err("ai_runtime_shutting_down".into()));
            }
            (
                state.harness_controls.values().cloned().collect::<Vec<_>>(),
                changed,
                tickets,
            )
        };
        self.retain_automation_shutdown_tickets(tickets);
        for control in controls {
            if let Err(error) = control.close_input() {
                lock(&self.inner).harness_retirement_error = Some(error);
            }
        }
        changed
    }

    pub(super) fn await_owned_harnesses(&self, persist_exit_changes: bool) {
        let mut state = lock(&self.inner);
        let deadline = Instant::now() + std::time::Duration::from_secs(20);
        while (!state.harness_processes.is_empty() || state.local_operations > 0)
            && state.harness_retirement_error.is_none()
        {
            if Instant::now() >= deadline {
                state.harness_retirement_error = Some("ai_harness_shutdown_unconfirmed".into());
                break;
            }
            state = self
                .harness_processes_completed
                .wait_timeout(state, std::time::Duration::from_millis(100))
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .0;
        }
        if state.server.is_some()
            && (persist_exit_changes || super::persistence_outcome_unknown(&state))
            && let Err(error) = maintenance::persist_quiescent_ledger(&mut state)
        {
            state
                .harness_retirement_error
                .get_or_insert_with(|| "ai_exit_persistence_unconfirmed".into());
            eprintln!("ai_exit_persistence_unconfirmed:{error}");
        }
        if let Some(error) = state.harness_retirement_error.as_ref() {
            eprintln!("ai_harness_shutdown_unconfirmed:{error}");
        }
        drop(state);
        let service = lock(&self.inner).codex_threads.take();
        if let Some(service) = service
            && let Err(error) = service.shutdown_checked()
        {
            lock(&self.inner).harness_retirement_error = Some(error);
        }
        self.finish_automation_shutdown_tickets();
    }
}

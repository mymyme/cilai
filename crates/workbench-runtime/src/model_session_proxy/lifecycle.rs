use super::ProxyState;
use crate::model_connections::ExecutionTransport;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

pub(super) struct RequestScopeMonitor {
    state: Arc<ProxyState>,
    revoked: tokio::sync::watch::Receiver<bool>,
    worker: tokio::task::JoinHandle<()>,
    _transport: Option<ExecutionTransport>,
}

impl RequestScopeMonitor {
    pub(super) fn start(state: Arc<ProxyState>, purpose: &'static str) -> Result<Self, String> {
        let transport = state
            .execution
            .as_ref()
            .map(|execution| execution.begin_transport(purpose))
            .transpose()?;
        let (sender, revoked) = tokio::sync::watch::channel(false);
        let checked_state = Arc::clone(&state);
        let worker = tokio::spawn(async move {
            loop {
                tokio::select! {
                    ()=sender.closed()=>return,
                    ()=tokio::time::sleep(Duration::from_millis(250))=>{}
                }
                let checked = Arc::clone(&checked_state);
                let authorized = tokio::task::spawn_blocking(move || {
                    !locally_revoked(&checked)
                        && checked.execution.as_ref().is_none_or(|execution| {
                            execution.validate_lifecycle().is_ok()
                                && execution.validate_request(purpose).is_ok()
                        })
                })
                .await
                .unwrap_or(false);
                if !authorized {
                    let _ = sender.send(true);
                    return;
                }
            }
        });
        Ok(Self {
            state,
            revoked,
            worker,
            _transport: transport,
        })
    }

    pub(super) async fn revoked(&mut self) {
        loop {
            if locally_revoked(&self.state) || *self.revoked.borrow() {
                return;
            }
            tokio::select! {
                changed=self.revoked.changed()=>if changed.is_err() {return;},
                ()=tokio::time::sleep(Duration::from_millis(250))=>{}
            }
        }
    }
}

fn locally_revoked(state: &ProxyState) -> bool {
    state.retired.load(Ordering::Acquire)
        || state
            .execution
            .as_ref()
            .is_some_and(crate::model_connections::ExecutionBundle::is_retiring)
}

impl Drop for RequestScopeMonitor {
    fn drop(&mut self) {
        self.worker.abort();
    }
}

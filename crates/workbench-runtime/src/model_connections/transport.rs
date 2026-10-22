use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

#[derive(Default)]
pub(super) struct TransportLifecycle {
    retiring: AtomicBool,
    active: Mutex<usize>,
    drained: Condvar,
}

pub(crate) struct ExecutionTransport(Arc<TransportLifecycle>);

impl TransportLifecycle {
    pub(super) fn enter(self: &Arc<Self>) -> Result<ExecutionTransport, String> {
        let mut active = self
            .active
            .lock()
            .map_err(|_| "model_execution_transport_state_unavailable")?;
        if self.is_retiring() {
            return Err("model_execution_retiring".into());
        }
        *active += 1;
        Ok(ExecutionTransport(Arc::clone(self)))
    }

    pub(super) fn is_retiring(&self) -> bool {
        self.retiring.load(Ordering::Acquire)
    }

    pub(super) fn retire_and_wait(&self, timeout: Duration) -> Result<(), String> {
        let mut active = self
            .active
            .lock()
            .map_err(|_| "model_execution_transport_state_unavailable")?;
        self.retiring.store(true, Ordering::Release);
        let deadline = Instant::now() + timeout;
        while *active != 0 {
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                return Err("model_execution_transport_exit_unconfirmed".into());
            };
            (active, _) = self
                .drained
                .wait_timeout(active, remaining)
                .map_err(|_| "model_execution_transport_state_unavailable")?;
        }
        Ok(())
    }
}

impl Drop for ExecutionTransport {
    fn drop(&mut self) {
        if let Ok(mut active) = self.0.active.lock() {
            *active -= 1;
            self.0.drained.notify_all();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retirement_requires_the_actual_transport_to_exit_and_keeps_admission_closed() {
        let lifecycle = Arc::new(TransportLifecycle::default());
        let request = lifecycle.enter().unwrap();
        assert_eq!(
            lifecycle.retire_and_wait(Duration::from_millis(10)),
            Err("model_execution_transport_exit_unconfirmed".into())
        );
        assert_eq!(
            lifecycle.enter().err().as_deref(),
            Some("model_execution_retiring")
        );
        drop(request);
        lifecycle.retire_and_wait(Duration::ZERO).unwrap();
    }
}

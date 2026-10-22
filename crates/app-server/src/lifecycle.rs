//! 服务关闭准入与真实客户端共同使用同一把锁，避免最后退出与新附着竞态。
use crate::SharedAppServerServices;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct AppServerLifecycle(Arc<Mutex<State>>);
struct State {
    stdio_open: bool,
    ipc_clients: u32,
    detached_requests: u32,
    closing: bool,
    fault: bool,
}
#[derive(Clone, Copy)]
pub enum LifecycleCaller {
    Stdio,
    Ipc,
    Disconnected,
}
pub struct LifecycleClient {
    lifecycle: AppServerLifecycle,
}
pub struct LifecycleRequest {
    lifecycle: AppServerLifecycle,
}
pub struct ShutdownDecision {
    pub can_close: bool,
    pub continuing: bool,
    pub reason_code: Option<&'static str>,
    pub ipc_client_count: u32,
    pub blocking_reasons: Vec<String>,
}
impl AppServerLifecycle {
    #[must_use]
    pub fn new(stdio_open: bool) -> Self {
        Self(Arc::new(Mutex::new(State {
            stdio_open,
            ipc_clients: 0,
            detached_requests: 0,
            closing: false,
            fault: false,
        })))
    }
    /// # Errors
    /// 关闭已经受理或计数异常时拒绝新客户端，不把它登记为可调用连接。
    pub fn admit_ipc(&self) -> Result<Option<LifecycleClient>, String> {
        let mut state = self
            .0
            .lock()
            .map_err(|_| "app_server_lifecycle_unconfirmed")?;
        if state.closing {
            return Ok(None);
        }
        state.ipc_clients = state
            .ipc_clients
            .checked_add(1)
            .ok_or("app_server_lifecycle_unconfirmed")?;
        Ok(Some(LifecycleClient {
            lifecycle: self.clone(),
        }))
    }
    /// # Errors
    /// 只在真实输入结束后撤销 stdio 客户端；不会释放任何业务写者。
    pub fn close_stdio(&self) -> Result<(), String> {
        self.0
            .lock()
            .map_err(|_| "app_server_lifecycle_unconfirmed")?
            .stdio_open = false;
        Ok(())
    }
    /// # Errors
    /// 关闭准入状态损坏时拒绝推断已经安全退出。
    pub fn is_closing(&self) -> Result<bool, String> {
        Ok(self
            .0
            .lock()
            .map_err(|_| "app_server_lifecycle_unconfirmed")?
            .closing)
    }
    /// # Errors
    /// 已开始关闭时不再受理脱离连接的业务任务。
    pub fn begin_detached_request(&self) -> Result<LifecycleRequest, String> {
        let mut state = self
            .0
            .lock()
            .map_err(|_| "app_server_lifecycle_unconfirmed")?;
        if state.closing {
            return Err("app_server_shutting_down".into());
        }
        state.detached_requests = state
            .detached_requests
            .checked_add(1)
            .ok_or("app_server_lifecycle_unconfirmed")?;
        Ok(LifecycleRequest {
            lifecycle: self.clone(),
        })
    }
    /// # Errors
    /// 计数、业务写者或持久状态不能核实时保持开放续管，不批准服务退出。
    pub fn shutdown_decision(
        &self,
        caller: LifecycleCaller,
        services: Option<&SharedAppServerServices>,
        commit: bool,
    ) -> Result<ShutdownDecision, String> {
        let mut state = self
            .0
            .lock()
            .map_err(|_| "app_server_lifecycle_unconfirmed")?;
        let mut result = ShutdownDecision {
            can_close: false,
            continuing: false,
            reason_code: None,
            ipc_client_count: state.ipc_clients,
            blocking_reasons: vec![],
        };
        if state.closing {
            result.reason_code = Some("app_server_shutting_down");
            return Ok(result);
        }
        let peers = match caller {
            LifecycleCaller::Stdio => state.ipc_clients,
            LifecycleCaller::Ipc => {
                state.ipc_clients.saturating_sub(1) + u32::from(state.stdio_open)
            }
            LifecycleCaller::Disconnected => state.ipc_clients + u32::from(state.stdio_open),
        };
        if peers > 0 {
            result.continuing = true;
            result.reason_code = Some("app_server_shared_clients_active");
            return Ok(result);
        }
        if state.fault || state.detached_requests > 0 {
            result
                .blocking_reasons
                .push("app_server_requests_active_or_unconfirmed".into());
        }
        if let Some(services) = services {
            let services = services
                .lock()
                .map_err(|_| "app_server_writers_unconfirmed")?;
            result
                .blocking_reasons
                .extend(services.shutdown_blocking_reasons()?);
        }
        if result.blocking_reasons.is_empty() {
            result.can_close = true;
            if commit {
                state.closing = true;
            }
        } else {
            result.reason_code = Some("app_server_writers_active");
        }
        Ok(result)
    }
}
impl Drop for LifecycleClient {
    fn drop(&mut self) {
        if let Ok(mut state) = self.lifecycle.0.lock() {
            if let Some(count) = state.ipc_clients.checked_sub(1) {
                state.ipc_clients = count;
            } else {
                state.fault = true;
            }
        }
    }
}
impl Drop for LifecycleRequest {
    fn drop(&mut self) {
        if let Ok(mut state) = self.lifecycle.0.lock() {
            if let Some(count) = state.detached_requests.checked_sub(1) {
                state.detached_requests = count;
            } else {
                state.fault = true;
            }
        }
    }
}

#[cfg(test)]
#[path = "lifecycle_tests.rs"]
mod tests;

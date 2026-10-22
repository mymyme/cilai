//! 需要联网、可能长时间等待的请求不占用请求通道。
//!
//! 一条连接上的请求原本按序处理：前一个请求处理完才读下一个。联网请求一旦等上数秒，
//! 排在后面的对话保存就会超过调用方的等待时限，调用方放弃后应用服务仍会执行那次保存，
//! 两边的状态修订由此错开。这里的请求改为：服务主锁内只读取本机状态做准备，联网放到
//! 独立线程完成，应答按请求编号单独回写，通道继续处理后续请求。

use crate::{
    AppServerServices, ConnectionRouter, Phase, RouteResult, ServiceError, SharedAppServerServices,
    invalid_params, limit_route_response, parse_params, unix_epoch_millis,
};
use contexthub_protocol::{
    ClientMessage, Frame, Request, RequestId, WorkspaceListDirectoryParams, business_error,
    parse_client_message, success,
};
use serde_json::{Value, json};

/// 在请求通道之外执行、产出完整应答文本的调用。
pub(crate) type DetachedCall = Box<dyn FnOnce() -> String + Send>;

impl ConnectionRouter {
    /// 是可以脱离请求通道执行的请求时，取出它的执行体；其余请求仍走原有的按序处理。
    pub(crate) fn detach(&self, frame: &Frame) -> Option<DetachedCall> {
        if self.phase != Phase::Ready {
            return None;
        }
        let Frame::Message(bytes) = frame else {
            return None;
        };
        let Ok(ClientMessage::Request(Request { id, method, params })) =
            parse_client_message(bytes)
        else {
            return None;
        };
        if self
            .personal_workspace_request_block(&method, params.as_ref())
            .is_some()
        {
            return None;
        }
        let services = self.services.clone()?;
        match method.as_str() {
            "workspace/listDirectory" => Some(detached::<WorkspaceListDirectoryParams, _, _>(
                id,
                params,
                services,
                "Workspace directory listing failed",
                AppServerServices::prepare_workspace_list_directory,
                crate::workspace_browse::execute_directory_read,
            )),
            "workspace/files" => {
                let completion_services = services.clone();
                Some(detached::<contexthub_protocol::WorkspaceFilesParams, _, _>(
                    id,
                    params,
                    services,
                    "Workspace files read failed",
                    AppServerServices::prepare_workspace_files,
                    move |prepared, params| {
                        crate::workspace_scan::execute_files(
                            &prepared,
                            params,
                            &completion_services,
                        )
                    },
                ))
            }
            "workspace/scan" => {
                let completion_services = services.clone();
                Some(detached::<contexthub_protocol::WorkspaceScanParams, _, _>(
                    id,
                    params,
                    services,
                    "Workspace scan failed",
                    AppServerServices::prepare_workspace_scan,
                    move |prepared, params| {
                        crate::workspace_scan::execute(&prepared, params, &completion_services)
                    },
                ))
            }
            _ => None,
        }
    }
}

fn detached<P, Prepared, Output>(
    id: RequestId,
    params: Option<Value>,
    services: SharedAppServerServices,
    message: &'static str,
    prepare: fn(&mut AppServerServices, &P, i64) -> Result<Prepared, ServiceError>,
    execute: impl FnOnce(Prepared, &P) -> Result<Output, ServiceError> + Send + 'static,
) -> DetachedCall
where
    P: serde::de::DeserializeOwned + 'static,
    Prepared: 'static,
    Output: serde::Serialize + 'static,
{
    Box::new(move || {
        let Some(params) = parse_params::<P>(params) else {
            return invalid_params(id, "invalid_method_params");
        };
        let now = match unix_epoch_millis() {
            Ok(now) => now,
            Err(reason) => {
                return business_error(id, reason, "System time is unavailable", json!({}));
            }
        };
        let prepared = prepare(
            &mut services
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            &params,
            now,
        );
        let response = match prepared.and_then(|prepared| execute(prepared, &params)) {
            Ok(result) => success(id, &result).expect("detached result serializes"),
            Err(error) => business_error(id, error.reason, message, json!({})),
        };
        limit_route_response(RouteResult {
            response: Some(response),
            should_exit: false,
        })
        .response
        .unwrap_or_default()
    })
}

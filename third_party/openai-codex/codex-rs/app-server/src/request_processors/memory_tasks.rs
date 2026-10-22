use crate::error_code::internal_error;
use codex_app_server_protocol::JSONRPCErrorError;

pub(super) type MemoryTask = tokio::task::JoinHandle<Result<(), String>>;

pub(super) async fn flush_memory_tasks(tasks: &mut [MemoryTask]) -> Result<(), JSONRPCErrorError> {
    let wait = async {
        let mut first_error = None;
        for task in tasks.iter_mut() {
            let result = task
                .await
                .map_err(|error| internal_error(format!("memory task failed: {error}")))
                .and_then(|result| result.map_err(internal_error));
            if let Err(error) = result {
                first_error.get_or_insert(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    };
    wait.await?;
    Ok(())
}

#[cfg(test)]
#[path = "memory_tasks_tests.rs"]
mod memory_flush_tests;

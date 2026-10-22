use contexthub_app_server::{
    AppServerLifecycle, ConnectionRouter, LifecycleCaller, SharedAppServerServices,
    serve_connection,
};
use contexthub_local_transport::{Endpoint, LocalListener};
use std::future::{Future, poll_fn};
use std::io;
use std::path::Path;
use std::pin::pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::task::Poll;
use tokio::sync::watch;
use tokio::task::JoinHandle;

pub struct LocalIpcServer {
    task: JoinHandle<io::Result<()>>,
}

impl LocalIpcServer {
    pub fn start(
        runtime_dir: &Path,
        server_instance_id: String,
        shutdown_tx: watch::Sender<bool>,
        shutdown_rx: watch::Receiver<bool>,
        lifecycle: AppServerLifecycle,
        services: SharedAppServerServices,
    ) -> io::Result<Self> {
        prepare_runtime_dir(runtime_dir)
            .map_err(|error| ipc_error("prepare runtime directory", &error))?;
        let endpoint = Endpoint::resolve(runtime_dir)
            .map_err(|error| ipc_error("resolve local IPC endpoint", &error))?;
        let listener = LocalListener::bind(&endpoint)
            .map_err(|error| ipc_error("bind local IPC endpoint", &error))?;
        let task = tokio::spawn(async move {
            accept_connections(
                listener,
                server_instance_id,
                shutdown_tx,
                shutdown_rx,
                lifecycle,
                services,
            )
            .await
        });
        Ok(Self { task })
    }

    pub async fn stop(self) -> io::Result<()> {
        self.task
            .await
            .map_err(|error| io::Error::other(format!("IPC task join failed: {error}")))?
    }
}

fn ipc_error(operation: &str, source: &io::Error) -> io::Error {
    io::Error::new(source.kind(), format!("{operation}: {source}"))
}

async fn accept_connections(
    listener: LocalListener,
    server_instance_id: String,
    shutdown_tx: watch::Sender<bool>,
    mut shutdown_rx: watch::Receiver<bool>,
    lifecycle: AppServerLifecycle,
    services: SharedAppServerServices,
) -> io::Result<()> {
    let sequence = Arc::new(AtomicU32::new(1));
    loop {
        let outcome = {
            let mut accept = pin!(listener.accept());
            let mut shutdown = pin!(shutdown_rx.changed());
            poll_fn(|context| {
                if let Poll::Ready(result) = shutdown.as_mut().poll(context) {
                    return Poll::Ready(AcceptOutcome::Shutdown(result));
                }
                accept.as_mut().poll(context).map(AcceptOutcome::Accepted)
            })
            .await
        };
        let accepted = match outcome {
            AcceptOutcome::Accepted(result) => Some(result?),
            AcceptOutcome::Shutdown(changed) => {
                changed.map_err(|_| {
                    io::Error::new(io::ErrorKind::BrokenPipe, "shutdown channel closed")
                })?;
                if *shutdown_rx.borrow() {
                    None
                } else {
                    continue;
                }
            }
        };
        let Some(connection) = accepted else {
            return Ok(());
        };
        let Some(client) = lifecycle.admit_ipc().map_err(io::Error::other)? else {
            continue;
        };
        let connection_number = sequence.fetch_add(1, Ordering::Relaxed);
        let router = ConnectionRouter::with_services(
            server_instance_id.clone(),
            format!("conn_uds_{connection_number}"),
            format!("cs_uds_{connection_number}"),
            true,
            services.clone(),
        )
        .with_lifecycle(lifecycle.clone(), LifecycleCaller::Ipc);
        let (reader, writer) = connection.into_split();
        let connection_shutdown_tx = shutdown_tx.clone();
        let connection_shutdown_rx = shutdown_rx.clone();
        tokio::spawn(async move {
            let _client = client;
            if let Err(error) = serve_connection(
                reader,
                writer,
                router,
                connection_shutdown_tx,
                connection_shutdown_rx,
            )
            .await
            {
                eprintln!("Local IPC connection failed: {error}");
            }
        });
    }
}

enum AcceptOutcome<T, S> {
    Accepted(T),
    Shutdown(S),
}

fn prepare_runtime_dir(runtime_dir: &Path) -> io::Result<()> {
    contexthub_platform_adapter::security::ensure_restricted_directory(runtime_dir)
}

#[cfg(all(test, unix))]
mod tests {
    #[cfg(unix)]
    use super::*;

    fn unique_root(label: &str) -> std::path::PathBuf {
        let suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "contexthub-ipc-{label}-{}-{suffix}",
            std::process::id()
        ))
    }

    #[test]
    fn runtime_directory_is_private_and_symlinks_are_rejected() {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;

        let root = unique_root("permissions");
        let runtime = root.join("run");
        prepare_runtime_dir(&runtime).unwrap();
        assert_eq!(
            fs::metadata(&runtime).unwrap().permissions().mode() & 0o777,
            0o700
        );
        fs::remove_dir(&runtime).unwrap();
        fs::remove_dir(&root).unwrap();

        let symlink_root = unique_root("symlink");
        let target = symlink_root.join("target");
        let link = symlink_root.join("run");
        fs::create_dir_all(&target).unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert_eq!(
            prepare_runtime_dir(&link).unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        fs::remove_file(&link).unwrap();
        fs::remove_dir(&target).unwrap();
        fs::remove_dir(&symlink_root).unwrap();
    }
}

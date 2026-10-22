mod instance;
mod local_ipc;
mod personal_profile;

use contexthub_app_server::{
    AppServerLifecycle, AppServerServices, ConnectionRouter, LifecycleCaller,
    SharedAppServerServices, serve_connection,
};
use contexthub_application::{
    DurableEventRepository, NewDurableEvent, TaskRepository, UnitOfWork,
    startup_recovery_transition,
};
use contexthub_persistence_sqlite::SqlitePersistence;
use contexthub_platform_adapter::instance::LifecycleMode;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::watch;

struct ServeOptions {
    local_ipc: Option<LocalIpcOptions>,
    transient_idle_timeout: Option<Duration>,
}

struct LocalIpcOptions {
    environment: String,
    data_dir: PathBuf,
    runtime_dir: PathBuf,
}

fn main() {
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    if arguments
        .first()
        .is_some_and(|value| value == "personal-data-restore")
    {
        let result = match arguments.as_slice() {
            [
                _,
                root_flag,
                root,
                operation_flag,
                operation,
                digest_flag,
                digest,
            ] if root_flag == "--business-root"
                && operation_flag == "--operation-id"
                && digest_flag == "--preview-digest" =>
            {
                match (operation.to_str(), digest.to_str()) {
                    (Some(operation), Some(digest)) => {
                        contexthub_app_server::personal_data::apply_restore_offline(
                            std::path::Path::new(root),
                            operation,
                            digest,
                        )
                    }
                    _ => Err("personal_data_request_invalid".into()),
                }
            }
            _ => Err("personal_data_request_invalid".into()),
        };
        match result {
            Ok(value) => println!("{value}"),
            Err(reason) => {
                eprintln!("{reason}");
                std::process::exit(16);
            }
        }
        return;
    }
    if arguments
        .first()
        .is_some_and(|value| value == contexthub_platform_adapter::writer_launch_gate::COMMAND)
    {
        if let Ok(code) = contexthub_platform_adapter::writer_launch_gate::run(&arguments[1..]) {
            std::process::exit(code);
        } else {
            eprintln!("personal_writer_launch_gate_failed");
            std::process::exit(17);
        }
    }
    if arguments
        .first()
        .is_some_and(|v| v == contexthub_platform_adapter::runtime_launch_gate::COMMAND)
    {
        match contexthub_platform_adapter::runtime_launch_gate::run(&arguments[1..]) {
            Ok(code) => std::process::exit(code),
            Err(error) => {
                eprintln!("personal_runtime_launch_gate_failed:{error}");
                std::process::exit(14);
            }
        }
    }
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("App Server runtime initialization failed: {error}");
            std::process::exit(20);
        }
    };
    runtime.block_on(async_main());
}

async fn async_main() {
    let options = match parse_options() {
        Ok(options) => options,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(10);
        }
    };
    if let Err((code, error)) = serve(options).await {
        eprintln!("App Server failure: {error}");
        std::process::exit(code);
    }
}

fn parse_options() -> Result<ServeOptions, &'static str> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [serve, transport, stdio]
            if serve == "serve" && transport == "--transport" && stdio == "stdio" =>
        {
            Ok(ServeOptions {
                local_ipc: None,
                transient_idle_timeout: None,
            })
        }
        [
            serve,
            transport,
            stdio,
            enable,
            environment_arg,
            environment,
            data_arg,
            data_dir,
            runtime_arg,
            runtime_dir,
        ] if serve == "serve"
            && transport == "--transport"
            && stdio == "stdio"
            && enable == "--enable-local-ipc"
            && environment_arg == "--environment"
            && data_arg == "--data-dir"
            && runtime_arg == "--runtime-dir" =>
        {
            Ok(ServeOptions {
                local_ipc: Some(LocalIpcOptions {
                    environment: environment.clone(),
                    data_dir: PathBuf::from(data_dir),
                    runtime_dir: PathBuf::from(runtime_dir),
                }),
                transient_idle_timeout: None,
            })
        }
        [
            serve,
            transport,
            local_ipc,
            transient,
            idle_arg,
            idle_ms,
            environment_arg,
            environment,
            data_arg,
            data_dir,
            runtime_arg,
            runtime_dir,
        ] if serve == "serve"
            && transport == "--transport"
            && local_ipc == "local-ipc"
            && transient == "--transient"
            && idle_arg == "--idle-timeout-ms"
            && environment_arg == "--environment"
            && data_arg == "--data-dir"
            && runtime_arg == "--runtime-dir" =>
        {
            let idle_ms = idle_ms
                .parse::<u64>()
                .map_err(|_| "idle timeout must be an integer number of milliseconds")?;
            if !(100..=300_000).contains(&idle_ms) {
                return Err("idle timeout must be between 100 and 300000 milliseconds");
            }
            Ok(ServeOptions {
                local_ipc: Some(LocalIpcOptions {
                    environment: environment.clone(),
                    data_dir: PathBuf::from(data_dir),
                    runtime_dir: PathBuf::from(runtime_dir),
                }),
                transient_idle_timeout: Some(Duration::from_millis(idle_ms)),
            })
        }
        _ => Err(
            "usage: contexthub-app-server serve --transport stdio [--enable-local-ipc --environment <name> --data-dir <path> --runtime-dir <path>] | contexthub-app-server serve --transport local-ipc --transient --idle-timeout-ms <100..300000> --environment <name> --data-dir <path> --runtime-dir <path>",
        ),
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "服务启动、后台任务与退出顺序集中管理"
)]
async fn serve(options: ServeOptions) -> Result<(), (i32, String)> {
    let instance_id = new_instance_id().map_err(|error| (20, error.to_string()))?;
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let lifecycle = AppServerLifecycle::new(options.transient_idle_timeout.is_none());

    let instance_guard = if let Some(local) = options.local_ipc.as_ref() {
        Some(
            instance::InstanceGuard::acquire(
                &local.data_dir,
                &local.runtime_dir,
                &local.environment,
                &instance_id,
                if options.transient_idle_timeout.is_some() {
                    LifecycleMode::Transient
                } else {
                    LifecycleMode::Owned
                },
            )
            .map_err(|error| (instance_exit_code(&error), error.to_string()))?,
        )
    } else {
        None
    };

    let services = initialize_services(options.local_ipc.as_ref(), &instance_id)?;
    if let Some(local) = options
        .local_ipc
        .as_ref()
        .filter(|_| instance_guard.is_some())
    {
        match contexthub_app_server::download_cleanup::cleanup(&local.data_dir) {
            Ok(report) => {
                for pending in report.pending {
                    eprintln!(
                        "download_cleanup_pending: {}: {}",
                        pending.journal.display(),
                        pending.reason
                    );
                }
            }
            Err(error) => eprintln!("download_cleanup_pending: {error}"),
        }
        if let Err(error) = services
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retire_material_cache()
        {
            eprintln!("retired_material_cache_cleanup_pending: {error}");
        }
    }

    let skill_worker = options.local_ipc.as_ref().map(|_| {
        contexthub_app_server::skill_generation::spawn(services.clone(), shutdown_rx.clone())
    });
    let capability_creation_worker = options.local_ipc.as_ref().map(|_| {
        contexthub_app_server::capability_generation::spawn(services.clone(), shutdown_rx.clone())
    });

    // 盯住工作区目录，磁盘变了就重扫。此前只有用户手动点"重新扫描"才更新索引，
    // 目录一旦重组，界面会长期显示一份早已不成立的文件树。
    //
    // 瞬时实例不盯：它只为一次调用而生、几秒后就退出，扫一遍工作区既来不及也没意义，
    // 而扫描期间会占住服务主锁，把那次调用本身拖到超时。
    let workspace_watch = options.transient_idle_timeout.is_none().then(|| {
        contexthub_app_server::workspace_watch::spawn_until_shutdown(
            services.clone(),
            shutdown_rx.clone(),
        )
    });

    let ipc_server = if let Some(local) = options.local_ipc.as_ref() {
        Some(
            local_ipc::LocalIpcServer::start(
                &local.runtime_dir,
                instance_id.clone(),
                shutdown_tx.clone(),
                shutdown_rx.clone(),
                lifecycle.clone(),
                services.clone(),
            )
            .map_err(|error| (14, error.to_string()))?,
        )
    } else {
        None
    };

    let result = if let Some(idle_timeout) = options.transient_idle_timeout {
        wait_for_lifecycle_exit(&lifecycle, &services, shutdown_rx.clone(), idle_timeout).await
    } else {
        let local_ipc_available = options.local_ipc.is_some();
        let router = ConnectionRouter::with_services(
            instance_id,
            "conn_stdio_1".to_owned(),
            "cs_stdio_1".to_owned(),
            local_ipc_available,
            services.clone(),
        )
        .with_lifecycle(lifecycle.clone(), LifecycleCaller::Stdio);
        let result = serve_connection(
            tokio::io::stdin(),
            tokio::io::stdout(),
            router,
            shutdown_tx.clone(),
            shutdown_rx.clone(),
        )
        .await;
        lifecycle.close_stdio().map_err(|reason| (20, reason))?;
        if options.local_ipc.is_some() {
            wait_for_lifecycle_exit(&lifecycle, &services, shutdown_rx, Duration::ZERO)
                .await
                .map_err(|error| (20, error.to_string()))?;
        }
        result
    };
    shutdown_tx.send_replace(true);

    let ipc_stop_result = if let Some(server) = ipc_server {
        server.stop().await
    } else {
        Ok(())
    };

    ipc_stop_result.map_err(|error| (20, error.to_string()))?;
    for worker in [skill_worker, capability_creation_worker, workspace_watch]
        .into_iter()
        .flatten()
    {
        worker
            .await
            .map_err(|error| (20, format!("background writer shutdown failed: {error}")))?;
    }
    drop(services);

    let instance_stop_result = if let Some(guard) = instance_guard {
        guard.stop()
    } else {
        Ok(())
    };

    instance_stop_result.map_err(|error| (20, error.to_string()))?;

    result.map_err(|error| (20, error.to_string()))
}

fn initialize_services(
    local: Option<&LocalIpcOptions>,
    instance_id: &str,
) -> Result<SharedAppServerServices, (i32, String)> {
    let upgrade = local
        .map(|local| personal_profile::UpgradeTarget::read(&local.data_dir))
        .transpose()
        .map_err(|error| (13, error))?
        .flatten();
    let mut persistence = local
        .map(|local| {
            SqlitePersistence::open(
                &local.data_dir.join("contexthub.db"),
                env!("CARGO_PKG_VERSION"),
            )
            .map_err(|error| (13, error.to_string()))
        })
        .transpose()?;
    if let Some(upgrade) = upgrade {
        upgrade
            .activate(
                persistence
                    .as_mut()
                    .ok_or_else(|| (13, "personal_profile_persistence_missing".into()))?,
                instance_id,
            )
            .map_err(|error| (13, error))?;
    }
    if let Some(local) = local {
        contexthub_persistence_sqlite::harness_home_migration::migrate(&local.data_dir)
            .map_err(|error| (13, error))?;
    }
    if let Some(persistence) = persistence.as_mut() {
        recover_startup_tasks(persistence, instance_id, unix_epoch_millis()?)
            .map_err(|error| (20, error))?;
    }
    Ok(Arc::new(Mutex::new(AppServerServices::with_data_dir(
        persistence,
        local.map(|options| options.data_dir.clone()),
    ))))
}

async fn wait_for_lifecycle_exit(
    lifecycle: &AppServerLifecycle,
    services: &SharedAppServerServices,
    mut shutdown_rx: watch::Receiver<bool>,
    idle_timeout: Duration,
) -> std::io::Result<()> {
    let mut idle_since = None;
    loop {
        if *shutdown_rx.borrow() {
            return Ok(());
        }
        let decision =
            lifecycle.shutdown_decision(LifecycleCaller::Disconnected, Some(services), false);
        if decision.is_ok_and(|decision| decision.can_close) {
            let since = idle_since.get_or_insert_with(std::time::Instant::now);
            if since.elapsed() >= idle_timeout
                && lifecycle
                    .shutdown_decision(LifecycleCaller::Disconnected, Some(services), true)
                    .is_ok_and(|decision| decision.can_close)
            {
                return Ok(());
            }
        } else {
            idle_since = None;
        }
        tokio::select! {
            changed = shutdown_rx.changed() => { changed.map_err(|_|std::io::Error::other("shutdown channel closed"))?; }
            () = tokio::time::sleep(Duration::from_millis(100)) => {}
        }
    }
}

fn new_instance_id() -> std::io::Result<String> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| std::io::Error::other(error.to_string()))?
        .as_nanos();
    Ok(format!("srv_{}_{nanos}", std::process::id()))
}

fn unix_epoch_millis() -> Result<i64, (i32, String)> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| (20, error.to_string()))?
        .as_millis();
    i64::try_from(millis).map_err(|_| (20, "system time exceeds SQLite INTEGER".to_owned()))
}

fn recover_startup_tasks(
    persistence: &mut SqlitePersistence,
    instance_id: &str,
    occurred_at: i64,
) -> Result<usize, String> {
    let tasks = persistence
        .transact(|repositories| repositories.nonterminal_tasks())
        .map_err(|error| format!("read startup recovery tasks: {error}"))?;
    let transitions = tasks
        .iter()
        .filter_map(
            |task| match startup_recovery_transition(task, occurred_at) {
                Ok(Some(transition)) => Some(Ok((task, transition))),
                Ok(None) => None,
                Err(error) => Some(Err(error.to_string())),
            },
        )
        .collect::<Result<Vec<_>, _>>()?;
    if transitions.is_empty() {
        return Ok(0);
    }
    persistence
        .transact(|repositories| {
            for (ordinal, (task, transition)) in transitions.iter().enumerate() {
                repositories.transition_task(transition)?;
                repositories.append_event(&NewDurableEvent {
                    event_id: contexthub_domain::EventId::new(format!(
                        "evt_recovery_{instance_id}_{ordinal}"
                    )),
                    workspace_id: task.workspace_id.clone(),
                    event_type: "task.startupRecovered".to_owned(),
                    object_type: Some("task".to_owned()),
                    object_id: Some(task.task_id.as_str().to_owned()),
                    payload_json: serde_json::json!({
                        "previousState": transition.expected_state.as_str(),
                        "state": transition.target_state.as_str(),
                        "reason": transition.error_reason,
                    })
                    .to_string(),
                    correlation_id: Some(task.correlation_id.clone()),
                    occurred_at,
                    created_at: occurred_at,
                })?;
            }
            Ok(())
        })
        .map_err(|error| format!("apply startup task recovery: {error}"))?;
    Ok(transitions.len())
}

fn instance_exit_code(error: &std::io::Error) -> i32 {
    match error.kind() {
        std::io::ErrorKind::AlreadyExists => 11,
        std::io::ErrorKind::WouldBlock => 12,
        _ => 15,
    }
}

#[cfg(test)]
mod recovery_tests {
    use super::*;
    use contexthub_application::{DurableEventReader, TaskRecord, TaskTransition};
    use contexthub_domain::{TaskAttemptId, TaskId, TaskState};
    use std::fs;

    #[test]
    fn startup_recovery_updates_only_running_tasks_and_emits_durable_events() {
        let fixture = TestDatabase::new("recover");
        let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        persistence
            .transact(|repositories| {
                seed_task(repositories, "scan-1", "scan", TaskState::Running)?;
                seed_task(repositories, "sync-1", "sync", TaskState::Running)?;
                seed_task(repositories, "queued-1", "scan", TaskState::Queued)?;
                seed_task(
                    repositories,
                    "waiting-1",
                    "preview",
                    TaskState::WaitingForUser,
                )?;
                Ok(())
            })
            .unwrap();

        assert_eq!(
            recover_startup_tasks(&mut persistence, "server-2", 20).unwrap(),
            2
        );
        let states = persistence
            .transact(|repositories| {
                ["scan-1", "sync-1", "queued-1", "waiting-1"]
                    .into_iter()
                    .map(|task_id| {
                        repositories
                            .task(&TaskId::new(task_id))
                            .map(|task| task.unwrap().state)
                    })
                    .collect::<Result<Vec<_>, _>>()
            })
            .unwrap();
        assert_eq!(
            states,
            vec![
                TaskState::Interrupted,
                TaskState::OutcomeUnknown,
                TaskState::Queued,
                TaskState::WaitingForUser,
            ]
        );
        let events = persistence.replay_global_after(0, 10).unwrap().events;
        assert_eq!(events.len(), 2);
        assert!(
            events
                .iter()
                .all(|event| event.event_type == "task.startupRecovered")
        );
    }

    #[test]
    fn unknown_running_task_blocks_recovery_without_partial_writes() {
        let fixture = TestDatabase::new("unknown");
        let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        persistence
            .transact(|repositories| {
                seed_task(repositories, "scan-1", "scan", TaskState::Running)?;
                seed_task(repositories, "unknown-1", "custom", TaskState::Running)
            })
            .unwrap();

        let error = recover_startup_tasks(&mut persistence, "server-2", 20).unwrap_err();
        assert!(error.contains("custom"));
        let states = persistence
            .transact(|repositories| {
                Ok((
                    repositories.task(&TaskId::new("scan-1"))?.unwrap().state,
                    repositories.task(&TaskId::new("unknown-1"))?.unwrap().state,
                ))
            })
            .unwrap();
        assert_eq!(states, (TaskState::Running, TaskState::Running));
        assert!(
            persistence
                .replay_global_after(0, 10)
                .unwrap()
                .events
                .is_empty()
        );
    }

    fn seed_task(
        repositories: &mut contexthub_persistence_sqlite::SqliteRepositoryTransaction<'_>,
        task_id: &str,
        task_type: &str,
        target_state: TaskState,
    ) -> Result<(), contexthub_persistence_sqlite::PersistenceError> {
        repositories.insert_task(&new_task(task_id, task_type))?;
        if target_state == TaskState::Queued {
            return Ok(());
        }
        repositories.start_task(
            &TaskId::new(task_id),
            &TaskAttemptId::new(format!("attempt-{task_id}")),
            "server-1",
            2,
        )?;
        if target_state == TaskState::WaitingForUser {
            repositories.transition_task(&TaskTransition {
                task_id: TaskId::new(task_id),
                expected_state: TaskState::Running,
                target_state,
                occurred_at: 3,
                result_ref_type: None,
                result_ref_id: None,
                error_reason: None,
                error_detail_json: None,
            })?;
        }
        Ok(())
    }

    fn new_task(task_id: &str, task_type: &str) -> TaskRecord {
        TaskRecord {
            task_id: TaskId::new(task_id),
            task_type: task_type.to_owned(),
            workspace_id: None,
            state: TaskState::Queued,
            progress_completed: 0,
            progress_total: None,
            current_stage: None,
            cancellable: true,
            payload_json: "{}".to_owned(),
            result_ref_type: None,
            result_ref_id: None,
            correlation_id: format!("correlation-{task_id}"),
            created_by_client: "desktop".to_owned(),
            created_at: 1,
            started_at: None,
            completed_at: None,
            heartbeat_at: None,
            error_reason: None,
            error_detail_json: None,
        }
    }

    struct TestDatabase {
        root: PathBuf,
        path: PathBuf,
    }

    impl TestDatabase {
        fn new(label: &str) -> Self {
            let root = PathBuf::from("/tmp").join(format!(
                "ch-startup-recovery-{label}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir(&root).unwrap();
            Self {
                path: root.join("contexthub.db"),
                root,
            }
        }
    }

    impl Drop for TestDatabase {
        fn drop(&mut self) {
            for suffix in ["-shm", "-wal", ""] {
                let path = PathBuf::from(format!("{}{suffix}", self.path.display()));
                if path.exists() {
                    fs::remove_file(path).unwrap();
                }
            }
            fs::remove_dir(&self.root).unwrap();
        }
    }
}

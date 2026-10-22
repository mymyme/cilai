use contexthub_local_transport::{
    Endpoint, EndpointIdentity, artifact_exists, artifact_exists_identity, cleanup_stale_identity,
    probe_liveness_identity,
};
use contexthub_persistence_sqlite::validate_no_active_writer;
use contexthub_platform_adapter::instance::{
    INSTANCE_SCHEMA_VERSION, InstanceDescriptor, LifecycleMode, hash_canonical_path,
    process_start_fingerprint, read_descriptor,
};
use contexthub_platform_adapter::security::{
    create_new_restricted_file, ensure_restricted_directory, open_restricted_file,
};
use contexthub_protocol::PROTOCOL_VERSION;
use std::fs::{self, File, TryLockError};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug)]
pub struct InstanceGuard {
    lock_file: Option<File>,
    record_path: PathBuf,
    server_instance_id: String,
}

impl InstanceGuard {
    pub fn acquire(
        data_dir: &Path,
        runtime_dir: &Path,
        environment: &str,
        server_instance_id: &str,
        lifecycle_mode: LifecycleMode,
    ) -> io::Result<Self> {
        validate_layout(data_dir, runtime_dir, environment)?;
        ensure_restricted_directory(data_dir)
            .map_err(|error| contextual_error("prepare instance data directory", &error))?;
        ensure_restricted_directory(runtime_dir)
            .map_err(|error| contextual_error("prepare instance runtime directory", &error))?;

        let data_dir_hash = hash_canonical_path(data_dir)?;
        let endpoint = Endpoint::resolve(runtime_dir)
            .map_err(|error| contextual_error("resolve local IPC endpoint", &error))?;
        let record_path = runtime_dir.join("instance.json");
        let lock_path = runtime_dir.join("instance.lock");
        let lock_file = open_restricted_file(&lock_path)
            .map_err(|error| contextual_error("open instance lock", &error))?;
        lock_file.try_lock().map_err(|error| match error {
            TryLockError::WouldBlock => io::Error::new(
                io::ErrorKind::AlreadyExists,
                "another App Server owns the instance lock",
            ),
            TryLockError::Error(source) => contextual_error("acquire instance lock", &source),
        })?;

        recover_stale_artifacts(
            data_dir,
            &record_path,
            &endpoint,
            environment,
            &data_dir_hash,
        )?;

        let record = InstanceDescriptor {
            schema_version: INSTANCE_SCHEMA_VERSION,
            pid: std::process::id(),
            process_start_fingerprint: process_start_fingerprint(std::process::id())?
                .ok_or_else(|| io::Error::other("current process fingerprint is unavailable"))?,
            server_instance_id: server_instance_id.to_owned(),
            lifecycle_mode,
            environment: environment.to_owned(),
            data_dir_hash,
            protocol_version: PROTOCOL_VERSION.to_owned(),
            ipc_endpoint: endpoint.identity().into_owned(),
            started_at_epoch_ms: unix_epoch_millis()?,
        };
        write_record_atomically(&record_path, &record)?;
        Ok(Self {
            lock_file: Some(lock_file),
            record_path,
            server_instance_id: server_instance_id.to_owned(),
        })
    }

    pub fn stop(mut self) -> io::Result<()> {
        let cleanup_result = self.remove_owned_record();
        let unlock_result = self.unlock();
        cleanup_result.and(unlock_result)
    }

    fn remove_owned_record(&mut self) -> io::Result<()> {
        let Some(record) = read_optional_record(&self.record_path)? else {
            return Ok(());
        };
        if record.server_instance_id != self.server_instance_id {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "instance record ownership changed while server was running",
            ));
        }
        fs::remove_file(&self.record_path)
            .map_err(|error| contextual_error("remove owned instance record", &error))
    }

    fn unlock(&mut self) -> io::Result<()> {
        let Some(file) = self.lock_file.take() else {
            return Ok(());
        };
        file.unlock()
            .map_err(|error| contextual_error("release instance lock", &error))
    }
}

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        if let Err(error) = self.remove_owned_record() {
            eprintln!("Instance record cleanup failed: {error}");
        }
        if let Err(error) = self.unlock() {
            eprintln!("Instance lock release failed: {error}");
        }
    }
}

fn validate_layout(data_dir: &Path, runtime_dir: &Path, environment: &str) -> io::Result<()> {
    if environment.is_empty()
        || !environment
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "environment must contain only lowercase ASCII letters, digits, or hyphens",
        ));
    }
    let legacy_layout = data_dir.file_name().and_then(|name| name.to_str()) == Some(environment);
    let profile_layout = data_dir
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(valid_profile_id)
        && data_dir
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            == Some("profiles")
        && data_dir
            .parent()
            .and_then(Path::parent)
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            == Some(environment);
    let personal_profile_layout = data_dir
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|id| {
            id.len() == 32
                && id
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        })
        && data_dir
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            == Some("profiles")
        && data_dir
            .parent()
            .and_then(Path::parent)
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            == Some("business")
        && data_dir
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            == Some("cn.cilai.personal.workbench");
    if !legacy_layout && !profile_layout && !personal_profile_layout {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "data directory must be the environment root or a managed profile",
        ));
    }
    if runtime_dir != data_dir.join("run") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "runtime directory must be the run directory inside the data directory",
        ));
    }
    Ok(())
}

fn valid_profile_id(value: &str) -> bool {
    value == "signed-out"
        || (value.len() == 32
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)))
}

fn recover_stale_artifacts(
    data_dir: &Path,
    record_path: &Path,
    endpoint: &Endpoint,
    environment: &str,
    data_dir_hash: &str,
) -> io::Result<()> {
    let record = read_optional_record(record_path)?;
    let Some(record) = record else {
        if artifact_exists(endpoint)
            .map_err(|error| contextual_error("inspect IPC endpoint", &error))?
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "IPC endpoint exists without a verifiable instance record",
            ));
        }
        return Ok(());
    };

    validate_existing_record(&record, endpoint, environment, data_dir_hash)?;
    let recorded_identity = EndpointIdentity::parse(&record.ipc_endpoint)
        .map_err(|error| contextual_error("parse recorded IPC endpoint", &error))?;
    let artifact_present = artifact_exists_identity(&recorded_identity)
        .map_err(|error| contextual_error("inspect recorded IPC endpoint", &error))?;
    if probe_liveness_identity(&recorded_identity)
        .map_err(|error| contextual_error("probe recorded IPC endpoint", &error))?
    {
        return Err(io::Error::new(
            io::ErrorKind::AddrInUse,
            "recorded IPC endpoint is still accepting connections",
        ));
    }
    match process_start_fingerprint(record.pid)? {
        None => {}
        Some(fingerprint) if fingerprint != record.process_start_fingerprint => {}
        Some(_) => {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "recorded process and start fingerprint are still active",
            ));
        }
    }
    validate_no_active_writer(&data_dir.join("contexthub.db")).map_err(|error| {
        io::Error::new(
            io::ErrorKind::WouldBlock,
            format!("database writer state cannot be confirmed: {error}"),
        )
    })?;
    if artifact_present {
        cleanup_stale_identity(&recorded_identity).map_err(|error| {
            contextual_error("remove validated stale recorded IPC endpoint", &error)
        })?;
    }
    fs::remove_file(record_path)
        .map_err(|error| contextual_error("remove validated stale instance record", &error))
}

fn validate_existing_record(
    record: &InstanceDescriptor,
    endpoint: &Endpoint,
    environment: &str,
    data_dir_hash: &str,
) -> io::Result<()> {
    if record.schema_version != INSTANCE_SCHEMA_VERSION
        || record.environment != environment
        || record.data_dir_hash != data_dir_hash
        || !endpoint.matches_persisted_identity(&record.ipc_endpoint)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "instance record does not match the requested environment and data directory",
        ));
    }
    Ok(())
}

fn read_optional_record(record_path: &Path) -> io::Result<Option<InstanceDescriptor>> {
    read_descriptor(record_path)
}

fn write_record_atomically(record_path: &Path, record: &InstanceDescriptor) -> io::Result<()> {
    let parent = record_path.parent().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "instance record has no parent")
    })?;
    let temporary_path = parent.join(format!("instance.json.tmp-{}", record.server_instance_id));
    let mut temporary = create_new_restricted_file(&temporary_path)
        .map_err(|error| contextual_error("create temporary instance record", &error))?;
    let bytes = serde_json::to_vec(record)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if let Err(error) = temporary
        .write_all(&bytes)
        .and_then(|()| temporary.sync_all())
    {
        let _ = fs::remove_file(&temporary_path);
        return Err(contextual_error("write temporary instance record", &error));
    }
    if let Err(error) = fs::rename(&temporary_path, record_path) {
        let _ = fs::remove_file(&temporary_path);
        return Err(contextual_error("publish instance record", &error));
    }
    contexthub_platform_adapter::filesystem::sync_directory(parent)
        .map_err(|error| contextual_error("sync instance runtime directory", &error))
}

fn unix_epoch_millis() -> io::Result<u64> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| io::Error::other(error.to_string()))?
        .as_millis();
    u64::try_from(millis).map_err(|_| io::Error::other("system time exceeds u64 milliseconds"))
}

fn contextual_error(operation: &str, source: &io::Error) -> io::Error {
    io::Error::new(source.kind(), format!("{operation}: {source}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_accepts_personal_profile_without_enterprise_environment_ownership() {
        let data = PathBuf::from(
            "/tmp/cn.cilai.personal.workbench/business/profiles/0123456789abcdef0123456789abcdef",
        );
        assert!(validate_layout(&data, &data.join("run"), "dev").is_ok());
        assert!(validate_layout(&data, &data.join("run"), "prod").is_ok());
        let invalid = PathBuf::from(
            "/tmp/com.cilai.workbench/business/profiles/0123456789abcdef0123456789abcdef",
        );
        assert!(validate_layout(&invalid, &invalid.join("run"), "dev").is_err());
    }

    #[test]
    fn layout_accepts_environment_root_and_managed_profiles() {
        let root = PathBuf::from("/tmp/com.cilai.workbench/dev");
        assert!(validate_layout(&root, &root.join("run"), "dev").is_ok());

        let signed_out = root.join("profiles/signed-out");
        assert!(validate_layout(&signed_out, &signed_out.join("run"), "dev").is_ok());

        let tenant = root.join("profiles/0123456789abcdef0123456789abcdef");
        assert!(validate_layout(&tenant, &tenant.join("run"), "dev").is_ok());
    }

    #[test]
    fn headless_session_keeps_the_environment_root_inside_isolated_host_data() {
        let temporary = std::env::temp_dir();
        let session = temporary.join("cilai-headless-layout-check");
        let host = session.join("host");
        let employee = temporary.join("employee-data");
        for environment in ["dev", "prod"] {
            let root = host.join(environment);
            assert_eq!(root.parent(), Some(host.as_path()));
            assert!(root.starts_with(&session));
            assert!(!root.starts_with(&employee));
            assert!(validate_layout(&root, &root.join("run"), environment).is_ok());
            let profile = root.join("profiles/signed-out");
            assert!(validate_layout(&profile, &profile.join("run"), environment).is_ok());
            assert!(validate_layout(&host, &host.join("run"), environment).is_err());
        }
    }

    #[test]
    fn layout_rejects_unmanaged_nested_directories() {
        let root = PathBuf::from("/tmp/com.cilai.workbench/dev");
        for data_dir in [
            root.join("signed-out"),
            root.join("profiles/other"),
            root.join("other/0123456789abcdef0123456789abcdef"),
        ] {
            let error = validate_layout(&data_dir, &data_dir.join("run"), "dev").unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        }
    }

    #[test]
    fn pid_reuse_is_recognized_by_start_fingerprint() {
        let root = unique_root("pid-reuse");
        let data_dir = root.join("test");
        let runtime_dir = data_dir.join("run");
        ensure_restricted_directory(&data_dir).unwrap();
        ensure_restricted_directory(&runtime_dir).unwrap();
        let record_path = runtime_dir.join("instance.json");
        let stale = test_record(&data_dir, &runtime_dir, "stale", "not-the-current-start");
        write_record_atomically(&record_path, &stale).unwrap();

        let guard = InstanceGuard::acquire(
            &data_dir,
            &runtime_dir,
            "test",
            "replacement-instance",
            LifecycleMode::Owned,
        )
        .unwrap();
        assert_eq!(
            read_optional_record(&record_path)
                .unwrap()
                .unwrap()
                .server_instance_id,
            "replacement-instance"
        );
        guard.stop().unwrap();
        cleanup(&root, &data_dir, &runtime_dir);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn stale_recovery_removes_the_validated_legacy_macos_socket() {
        let root = unique_root("legacy-macos-socket");
        let data_dir = root.join("test");
        let runtime_dir = data_dir.join("run");
        ensure_restricted_directory(&data_dir).unwrap();
        ensure_restricted_directory(&runtime_dir).unwrap();
        let legacy_socket = runtime_dir.join("c.sock");
        let listener = std::os::unix::net::UnixListener::bind(&legacy_socket).unwrap();
        drop(listener);

        let record_path = runtime_dir.join("instance.json");
        let mut stale = test_record(
            &data_dir,
            &runtime_dir,
            "legacy-stale",
            "not-the-current-start",
        );
        stale.ipc_endpoint = legacy_socket.to_string_lossy().into_owned();
        write_record_atomically(&record_path, &stale).unwrap();

        let guard = InstanceGuard::acquire(
            &data_dir,
            &runtime_dir,
            "test",
            "replacement-instance",
            LifecycleMode::Owned,
        )
        .unwrap();
        assert!(!legacy_socket.exists());
        assert_eq!(
            read_optional_record(&record_path)
                .unwrap()
                .unwrap()
                .ipc_endpoint,
            Endpoint::resolve(&runtime_dir).unwrap().identity().as_ref()
        );

        guard.stop().unwrap();
        cleanup(&root, &data_dir, &runtime_dir);
    }

    #[test]
    fn malformed_record_and_unverifiable_socket_are_not_deleted() {
        let root = unique_root("unverifiable");
        let data_dir = root.join("test");
        let runtime_dir = data_dir.join("run");
        ensure_restricted_directory(&data_dir).unwrap();
        ensure_restricted_directory(&runtime_dir).unwrap();
        let record_path = runtime_dir.join("instance.json");
        let mut record = create_new_restricted_file(&record_path).unwrap();
        record.write_all(b"not-json").unwrap();
        drop(record);
        let error =
            InstanceGuard::acquire(&data_dir, &runtime_dir, "test", "new", LifecycleMode::Owned)
                .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(fs::read(&record_path).unwrap(), b"not-json");
        fs::remove_file(&record_path).unwrap();

        #[cfg(unix)]
        {
            let socket_path =
                PathBuf::from(Endpoint::resolve(&runtime_dir).unwrap().identity().as_ref());
            let listener = std::os::unix::net::UnixListener::bind(&socket_path).unwrap();
            drop(listener);
            let error = InstanceGuard::acquire(
                &data_dir,
                &runtime_dir,
                "test",
                "new",
                LifecycleMode::Owned,
            )
            .unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            assert!(socket_path.exists());
            fs::remove_file(socket_path).unwrap();
        }
        cleanup(&root, &data_dir, &runtime_dir);
    }

    #[test]
    fn stale_recovery_accepts_a_database_after_writer_validation() {
        let root = unique_root("database");
        let data_dir = root.join("test");
        let runtime_dir = data_dir.join("run");
        ensure_restricted_directory(&data_dir).unwrap();
        ensure_restricted_directory(&runtime_dir).unwrap();
        let record_path = runtime_dir.join("instance.json");
        let stale = test_record(&data_dir, &runtime_dir, "stale", "not-the-current-start");
        write_record_atomically(&record_path, &stale).unwrap();
        fs::write(data_dir.join("contexthub.db"), b"").unwrap();

        let guard =
            InstanceGuard::acquire(&data_dir, &runtime_dir, "test", "new", LifecycleMode::Owned)
                .unwrap();
        assert_eq!(
            read_optional_record(&record_path)
                .unwrap()
                .unwrap()
                .server_instance_id,
            "new"
        );
        guard.stop().unwrap();
        fs::remove_file(data_dir.join("contexthub.db")).unwrap();
        cleanup(&root, &data_dir, &runtime_dir);
    }

    #[test]
    fn stale_recovery_preserves_an_unverifiable_database() {
        let root = unique_root("invalid-database");
        let data_dir = root.join("test");
        let runtime_dir = data_dir.join("run");
        ensure_restricted_directory(&data_dir).unwrap();
        ensure_restricted_directory(&runtime_dir).unwrap();
        let record_path = runtime_dir.join("instance.json");
        let stale = test_record(&data_dir, &runtime_dir, "stale", "not-the-current-start");
        write_record_atomically(&record_path, &stale).unwrap();
        fs::write(data_dir.join("contexthub.db"), b"not-a-sqlite-database").unwrap();

        let error =
            InstanceGuard::acquire(&data_dir, &runtime_dir, "test", "new", LifecycleMode::Owned)
                .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        assert!(record_path.exists());
        fs::remove_file(data_dir.join("contexthub.db")).unwrap();
        fs::remove_file(record_path).unwrap();
        cleanup(&root, &data_dir, &runtime_dir);
    }

    fn test_record(
        data_dir: &Path,
        runtime_dir: &Path,
        instance_id: &str,
        fingerprint: &str,
    ) -> InstanceDescriptor {
        InstanceDescriptor {
            schema_version: INSTANCE_SCHEMA_VERSION,
            pid: std::process::id(),
            process_start_fingerprint: fingerprint.to_owned(),
            server_instance_id: instance_id.to_owned(),
            lifecycle_mode: LifecycleMode::Owned,
            environment: "test".to_owned(),
            data_dir_hash: hash_canonical_path(data_dir).unwrap(),
            protocol_version: PROTOCOL_VERSION.to_owned(),
            ipc_endpoint: Endpoint::resolve(runtime_dir)
                .unwrap()
                .identity()
                .into_owned(),
            started_at_epoch_ms: unix_epoch_millis().unwrap(),
        }
    }

    fn unique_root(label: &str) -> PathBuf {
        PathBuf::from("/tmp").join(format!(
            "ch-instance-unit-{label}-{}-{}",
            std::process::id(),
            unix_epoch_millis().unwrap()
        ))
    }

    fn cleanup(root: &Path, data_dir: &Path, runtime_dir: &Path) {
        fs::remove_file(runtime_dir.join("instance.lock")).unwrap();
        fs::remove_dir(runtime_dir).unwrap();
        fs::remove_dir(data_dir).unwrap();
        fs::remove_dir(root).unwrap();
    }
}

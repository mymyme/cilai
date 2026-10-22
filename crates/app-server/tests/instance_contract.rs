use contexthub_local_transport::{Endpoint, artifact_exists, probe_liveness};
use contexthub_platform_adapter::security::verify_restricted_path;
use serde_json::Value;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

struct RunningServer {
    child: Child,
    stdin: ChildStdin,
    reader: BufReader<ChildStdout>,
}

#[test]
fn second_instance_is_rejected_without_disrupting_the_owner() {
    let root = unique_root("exclusive");
    let data_dir = root.join("test");
    let mut owner = spawn_server(&data_dir, "test");
    let owner_initialize = initialize(&mut owner, "owner-init");
    assert_eq!(owner_initialize["result"]["capabilities"]["localIpc"], true);
    assert_instance_record(&data_dir, &owner_initialize);

    let contender = server_command(&data_dir, "test")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("contender starts");
    assert_eq!(contender.status.code(), Some(11));
    assert!(contender.stdout.is_empty());
    assert!(
        String::from_utf8(contender.stderr)
            .unwrap()
            .contains("another App Server owns the instance lock")
    );

    ready_and_ping(&mut owner, "owner-ping");
    assert_eq!(
        read_jsonl(&mut owner.reader, "owner ping")["id"],
        "owner-ping"
    );
    shutdown(owner);
    assert_clean_shutdown(&data_dir);
    cleanup_data_dir(&root, &data_dir);
}

#[test]
fn crash_artifacts_are_recovered_only_after_the_owner_is_gone() {
    let root = unique_root("crash");
    let data_dir = root.join("test");
    let mut crashed = spawn_server(&data_dir, "test");
    let first = initialize(&mut crashed, "first-init");
    let first_instance = first["result"]["server"]["instanceId"]
        .as_str()
        .unwrap()
        .to_owned();
    crashed.child.kill().expect("kill owned test server");
    crashed.child.wait().expect("reap killed test server");
    drop(crashed.stdin);
    drop(crashed.reader);
    assert!(data_dir.join("run/instance.json").is_file());
    // 被强制结束的实例已经不再接受连接，接管方必须据此判定其为陈旧记录。两端的文件系统
    // 残留形态不同（Unix 留下套接字文件、Windows 命名管道随进程消失），但对外暴露的
    // 存活判定一致，所以这里断言存活性而不是断言残留文件。
    assert!(!probe_liveness(&Endpoint::resolve(&data_dir.join("run")).unwrap()).unwrap());

    let mut replacement = spawn_server(&data_dir, "test");
    let second = initialize(&mut replacement, "second-init");
    assert_ne!(second["result"]["server"]["instanceId"], first_instance);
    shutdown(replacement);
    assert_clean_shutdown(&data_dir);
    cleanup_data_dir(&root, &data_dir);
}

#[test]
fn environments_hold_independent_locks_and_endpoints() {
    let root = unique_root("environments");
    let dev_dir = root.join("dev");
    let prod_dir = root.join("prod");
    let mut dev = spawn_server(&dev_dir, "dev");
    let mut prod = spawn_server(&prod_dir, "prod");
    let dev_initialize = initialize(&mut dev, "dev-init");
    let prod_initialize = initialize(&mut prod, "prod-init");
    assert_ne!(
        dev_initialize["result"]["server"]["instanceId"],
        prod_initialize["result"]["server"]["instanceId"]
    );
    // 两个环境各自派生出互不冲突的端点标识，并各自写下自己的实例记录。这里不做连通性
    // 探测：探测会真的建立一次连接，对正在服务的实例有侵入性，会干扰随后的正常退出。
    let dev_endpoint = Endpoint::resolve(&dev_dir.join("run")).unwrap();
    let prod_endpoint = Endpoint::resolve(&prod_dir.join("run")).unwrap();
    assert_ne!(dev_endpoint.identity(), prod_endpoint.identity());
    assert!(dev_dir.join("run/instance.json").is_file());
    assert!(prod_dir.join("run/instance.json").is_file());
    shutdown(dev);
    shutdown(prod);
    cleanup_data_dir(&root, &dev_dir);
    cleanup_data_dir(&root, &prod_dir);
}

#[test]
fn managed_profile_completes_the_stdio_startup_handshake() {
    let root = unique_root("managed-profile");
    let environment_dir = root.join("test");
    let data_dir = environment_dir.join("profiles/signed-out");
    let mut server = spawn_server(&data_dir, "test");
    let initialized = initialize(&mut server, "profile-init");
    assert_eq!(initialized["result"]["capabilities"]["localIpc"], true);
    assert_instance_record(&data_dir, &initialized);

    shutdown(server);
    assert_clean_shutdown(&data_dir);
    cleanup_data_dir(&root, &data_dir);
    fs::remove_dir(environment_dir.join("profiles")).unwrap();
    fs::remove_dir(environment_dir).unwrap();
    fs::remove_dir(root).unwrap();
}

fn spawn_server(data_dir: &Path, environment: &str) -> RunningServer {
    let mut child = server_command(data_dir, environment)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("App Server starts");
    let stdin = child.stdin.take().expect("stdin available");
    let reader = BufReader::new(child.stdout.take().expect("stdout available"));
    RunningServer {
        child,
        stdin,
        reader,
    }
}

fn server_command(data_dir: &Path, environment: &str) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_contexthub-app-server"));
    command
        .args([
            "serve",
            "--transport",
            "stdio",
            "--enable-local-ipc",
            "--environment",
            environment,
            "--data-dir",
        ])
        .arg(data_dir)
        .arg("--runtime-dir")
        .arg(data_dir.join("run"));
    command
}

fn initialize(server: &mut RunningServer, id: &str) -> Value {
    writeln!(server.stdin, r#"{{"jsonrpc":"2.0","id":"{id}","method":"initialize","params":{{"client":{{"name":"instance-test","version":"0.1.0","platform":"macos","arch":"arm64"}},"protocol":{{"minVersion":"1.0","maxVersion":"1.0"}}}}}}"#).unwrap();
    server.stdin.flush().unwrap();
    let mut line = String::new();
    let count = server.reader.read_line(&mut line).expect("read initialize");
    if count == 0 {
        let mut stderr = String::new();
        server
            .child
            .stderr
            .as_mut()
            .expect("stderr available")
            .read_to_string(&mut stderr)
            .expect("read startup stderr");
        panic!("App Server closed during initialize: {stderr}");
    }
    serde_json::from_str(&line).expect("initialize line must be JSON")
}

fn ready_and_ping(server: &mut RunningServer, id: &str) {
    writeln!(
        server.stdin,
        r#"{{"jsonrpc":"2.0","method":"initialized","params":{{}}}}"#
    )
    .unwrap();
    writeln!(
        server.stdin,
        r#"{{"jsonrpc":"2.0","id":"{id}","method":"ping"}}"#
    )
    .unwrap();
    server.stdin.flush().unwrap();
}

fn shutdown(mut server: RunningServer) {
    writeln!(
        server.stdin,
        r#"{{"jsonrpc":"2.0","id":"prepare","method":"shutdown/prepare"}}"#
    )
    .unwrap();
    server.stdin.flush().unwrap();
    assert_eq!(
        read_jsonl(&mut server.reader, "shutdown prepare")["result"]["ready"],
        true
    );
    writeln!(
        server.stdin,
        r#"{{"jsonrpc":"2.0","id":"execute","method":"shutdown/execute"}}"#
    )
    .unwrap();
    server.stdin.flush().unwrap();
    assert_eq!(
        read_jsonl(&mut server.reader, "shutdown execute")["result"]["accepted"],
        true
    );
    drop(server.stdin);
    assert!(server.child.wait().expect("server exits").success());
}

fn assert_instance_record(data_dir: &Path, initialize: &Value) {
    let record_path = data_dir.join("run/instance.json");
    // 实例记录必须落在受限文件契约内：macOS 校验 0600 权限位，Windows 校验仅当前用户与
    // SYSTEM 的访问控制列表，两端由同一个判定接口负责。
    verify_restricted_path(&record_path, false).unwrap();
    let record: Value = serde_json::from_slice(&fs::read(record_path).unwrap()).unwrap();
    assert_eq!(record["schemaVersion"], 1);
    assert_eq!(
        record["serverInstanceId"],
        initialize["result"]["server"]["instanceId"]
    );
    assert_eq!(record["environment"], "test");
    assert_eq!(record["protocolVersion"], "1.0");
    assert_eq!(record["dataDirHash"].as_str().unwrap().len(), 64);
    assert!(record.get("token").is_none());
}

fn assert_clean_shutdown(data_dir: &Path) {
    // 干净退出后本地端点既不该留下文件系统残留，也不该继续接受连接。Windows 命名管道
    // 本就不落地为文件，残留判定在该端恒为假，因此连通性探测才是两端都有信号的断言。
    let endpoint = Endpoint::resolve(&data_dir.join("run")).unwrap();
    assert!(!artifact_exists(&endpoint).unwrap());
    assert!(!probe_liveness(&endpoint).unwrap());
    assert!(!data_dir.join("run/instance.json").exists());
    assert!(data_dir.join("run/instance.lock").is_file());
    assert!(data_dir.join("contexthub.db").is_file());
}

fn cleanup_data_dir(root: &Path, data_dir: &Path) {
    fs::remove_file(data_dir.join("contexthub.db")).unwrap();
    fs::remove_file(data_dir.join("run/instance.lock")).unwrap();
    fs::remove_dir(data_dir.join("run")).unwrap();
    fs::remove_dir(data_dir).unwrap();
    if root.read_dir().unwrap().next().is_none() {
        fs::remove_dir(root).unwrap();
    }
}

fn read_jsonl(reader: &mut impl BufRead, stage: &str) -> Value {
    let mut line = String::new();
    let count = reader.read_line(&mut line).expect("read response");
    assert!(count > 0, "App Server closed connection during {stage}");
    serde_json::from_str(&line).expect("line must be JSON")
}

fn unique_root(label: &str) -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    PathBuf::from("/tmp").join(format!(
        "ch-instance-{label}-{}-{suffix}",
        std::process::id()
    ))
}

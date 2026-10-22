use contexthub_local_transport::{Endpoint, LocalStream, artifact_exists, probe_liveness};
use contexthub_platform_adapter::security::verify_restricted_path;
use serde_json::Value;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn stdio_and_local_ipc_share_one_server_and_protocol_contract() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    // Unix 域套接字路径有约 104 字符上限，macOS 的 $TMPDIR 是很长的 /var/folders/…，
    // 套上进程号会直接超限；因此优先用短路径 /tmp，没有该目录的平台才回退系统临时目录。
    let base = if Path::new("/tmp").is_dir() {
        PathBuf::from("/tmp")
    } else {
        std::env::temp_dir()
    };
    let root = base.join(format!("ch-localipc-{}-{suffix}", std::process::id()));
    let data_dir = root.join("test");
    let runtime = data_dir.join("run");
    let mut child = spawn_server(&data_dir, &runtime);
    let mut stdin = child.stdin.take().expect("stdin available");
    let mut stdio_reader = BufReader::new(child.stdout.take().expect("stdout available"));

    initialize(&mut stdin, "stdio-init");
    let stdio_initialize = read_jsonl(&mut stdio_reader, "stdio initialize");
    assert_eq!(stdio_initialize["result"]["capabilities"]["localIpc"], true);
    let server_instance = stdio_initialize["result"]["server"]["instanceId"]
        .as_str()
        .expect("server instance id")
        .to_owned();
    // 运行时目录的受限强度由统一接口校验：macOS 比对 0700 权限位，Windows 比对访问控制列表。
    verify_restricted_path(&runtime, true).unwrap();
    assert_unix_socket_shape(&runtime);

    // 注意：跨平台连接句柄尚未提供读写超时设置，这里去掉了原先的 2 秒超时兜底；用例本身
    // 每次写入后都紧跟一次预期响应的读取，卡住会表现为用例挂起而非静默通过。
    let endpoint = Endpoint::resolve(&runtime).expect("endpoint resolves");
    let mut uds = LocalStream::connect(&endpoint).expect("same-user local connection accepted");
    let mut uds_reader = BufReader::new(uds.try_clone().expect("clone local reader"));
    initialize(&mut uds, "uds-init");
    let uds_initialize = read_jsonl(&mut uds_reader, "local IPC initialize");
    assert_eq!(
        uds_initialize["result"]["server"]["instanceId"],
        server_instance
    );
    assert!(
        uds_initialize["result"]["session"]["connectionId"]
            .as_str()
            .unwrap()
            .starts_with("conn_uds_")
    );

    ready_and_ping(&mut uds, "uds-ping");
    assert_eq!(
        read_jsonl(&mut uds_reader, "local IPC ping")["id"],
        "uds-ping"
    );
    ready_and_ping(&mut stdin, "stdio-ping");
    assert_eq!(
        read_jsonl(&mut stdio_reader, "stdio ping")["id"],
        "stdio-ping"
    );

    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":"prepare","method":"shutdown/prepare"}}"#
    )
    .unwrap();
    stdin.flush().unwrap();
    let continued = read_jsonl(&mut stdio_reader, "shutdown prepare");
    assert_eq!(continued["result"]["ready"], false);
    assert_eq!(continued["result"]["continuing"], true);
    assert_eq!(
        continued["result"]["reasonCode"],
        "app_server_shared_clients_active"
    );
    assert_eq!(continued["result"]["serverInstanceId"], server_instance);
    drop(stdin);
    writeln!(
        uds,
        r#"{{"jsonrpc":"2.0","id":"after-owner-eof","method":"ping"}}"#
    )
    .unwrap();
    uds.flush().unwrap();
    assert_eq!(
        read_jsonl(&mut uds_reader, "continued IPC after stdio EOF")["id"],
        "after-owner-eof"
    );
    drop(uds_reader);
    drop(uds);

    assert!(child.wait().expect("App Server exits").success());
    assert!(
        !artifact_exists(&endpoint).unwrap(),
        "owned endpoint artifact is removed on clean shutdown"
    );
    assert!(
        !probe_liveness(&endpoint).unwrap(),
        "endpoint stops accepting connections on clean shutdown"
    );
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    assert!(stderr.is_empty(), "unexpected stderr: {stderr}");
    fs::remove_file(data_dir.join("contexthub.db")).unwrap();
    fs::remove_file(runtime.join("instance.lock")).unwrap();
    fs::remove_dir(&runtime).unwrap();
    fs::remove_dir(&data_dir).unwrap();
    fs::remove_dir(&root).unwrap();
}

/// 套接字的文件形态与权限位是 Unix 专属的落地方式：Windows 命名管道不落地为文件，其访问
/// 控制在创建时设定且没有对外的读回接口，因此这段断言只在 Unix 上成立，其余平台为空实现。
#[cfg(unix)]
fn assert_unix_socket_shape(runtime: &Path) {
    use std::os::unix::fs::{FileTypeExt, PermissionsExt};
    let endpoint = Endpoint::resolve(runtime).expect("endpoint resolves for socket inspection");
    let socket = PathBuf::from(endpoint.identity().as_ref());
    let socket_metadata = fs::metadata(&socket).expect("socket created before initialize response");
    assert!(socket_metadata.file_type().is_socket());
    assert_eq!(socket_metadata.permissions().mode() & 0o777, 0o600);
}

#[cfg(not(unix))]
fn assert_unix_socket_shape(_runtime: &Path) {}

fn spawn_server(data_dir: &PathBuf, runtime: &PathBuf) -> std::process::Child {
    Command::new(env!("CARGO_BIN_EXE_contexthub-app-server"))
        .args([
            "serve",
            "--transport",
            "stdio",
            "--enable-local-ipc",
            "--environment",
            "test",
            "--data-dir",
        ])
        .arg(data_dir)
        .args(["--runtime-dir"])
        .arg(runtime)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("App Server starts")
}

fn initialize(writer: &mut impl Write, id: &str) {
    writeln!(writer, r#"{{"jsonrpc":"2.0","id":"{id}","method":"initialize","params":{{"client":{{"name":"contract-test","version":"0.1.0","platform":"macos","arch":"arm64"}},"protocol":{{"minVersion":"1.0","maxVersion":"1.0"}}}}}}"#).unwrap();
    writer.flush().unwrap();
}

fn ready_and_ping(writer: &mut impl Write, id: &str) {
    writeln!(
        writer,
        r#"{{"jsonrpc":"2.0","method":"initialized","params":{{}}}}"#
    )
    .unwrap();
    writeln!(writer, r#"{{"jsonrpc":"2.0","id":"{id}","method":"ping"}}"#).unwrap();
    writer.flush().unwrap();
}

fn read_jsonl(reader: &mut impl BufRead, stage: &str) -> Value {
    let mut line = String::new();
    let count = reader.read_line(&mut line).expect("read response");
    assert!(count > 0, "App Server closed connection during {stage}");
    assert!(line.ends_with('\n'));
    serde_json::from_str(&line).expect("line must be one JSON value")
}

use contexthub_local_transport::{Endpoint, LocalStream, artifact_exists, probe_liveness};
use serde_json::Value;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[test]
fn transient_instance_stays_alive_for_a_client_then_exits_when_idle() {
    let root = unique_root("connected-idle");
    let data_dir = root.join("test");
    let mut child = spawn_transient(&data_dir, 200);
    let endpoint = wait_for_endpoint(&data_dir, &mut child);

    let record: Value = serde_json::from_slice(
        &fs::read(data_dir.join("run/instance.json")).expect("instance record is readable"),
    )
    .expect("instance record is JSON");
    assert_eq!(record["lifecycleMode"], "transient");

    let mut stream = connect_when_ready(&endpoint, Duration::from_secs(5));
    let reader_stream = stream.try_clone().expect("client stream clones");
    let mut reader = BufReader::new(reader_stream);
    writeln!(stream, r#"{{"jsonrpc":"2.0","id":"init","method":"initialize","params":{{"client":{{"name":"transient-test","version":"0.1.0","platform":"macos","arch":"arm64"}},"protocol":{{"minVersion":"1.0","maxVersion":"1.0"}}}}}}"#).unwrap();
    stream.flush().unwrap();
    let mut line = String::new();
    reader.read_line(&mut line).expect("initialize responds");
    assert_eq!(serde_json::from_str::<Value>(&line).unwrap()["id"], "init");

    thread::sleep(Duration::from_millis(350));
    assert!(child.try_wait().unwrap().is_none());
    drop(reader);
    drop(stream);

    wait_for_successful_exit(&mut child, Duration::from_secs(2));
    assert_clean_shutdown(&data_dir);
    cleanup(&root, &data_dir);
}

#[test]
fn unclaimed_transient_instance_exits_after_startup_idle_timeout() {
    let root = unique_root("unclaimed-idle");
    let data_dir = root.join("test");
    let mut child = spawn_transient(&data_dir, 200);
    wait_for_endpoint(&data_dir, &mut child);
    wait_for_successful_exit(&mut child, Duration::from_secs(2));
    assert_clean_shutdown(&data_dir);
    cleanup(&root, &data_dir);
}

/// 这个场景要让启动方进程先行退出、瞬时实例继续存活，派生游离进程依赖 POSIX shell 的
/// 后台作业语法，因此测试脚手架本身仍是 Unix 专属；被验证的瞬时实例生命周期本身与平台
/// 无关，Windows 端等价的派生方式待真机阶段补齐。
#[cfg(unix)]
#[test]
fn transient_instance_survives_the_launcher_process_exit() {
    let root = unique_root("launcher-exit");
    let data_dir = root.join("test");
    let runtime_dir = data_dir.join("run");
    let launcher = Command::new("/bin/sh")
        .args([
            "-c",
            r#""$1" serve --transport local-ipc --transient --idle-timeout-ms 200 --environment test --data-dir "$2" --runtime-dir "$3" </dev/null >/dev/null 2>/dev/null &"#,
            "transient-launcher",
            env!("CARGO_BIN_EXE_contexthub-app-server"),
        ])
        .arg(&data_dir)
        .arg(&runtime_dir)
        .status()
        .expect("launcher process runs");
    assert!(launcher.success());

    wait_for_path(&data_dir.join("run/instance.json"), Duration::from_secs(5));
    let endpoint = Endpoint::resolve(&runtime_dir).expect("endpoint resolves after publication");
    let mut stream = connect_when_ready(&endpoint, Duration::from_secs(5));
    writeln!(stream, r#"{{"jsonrpc":"2.0","id":"init","method":"initialize","params":{{"client":{{"name":"launcher-exit-test","version":"0.1.0","platform":"macos","arch":"arm64"}},"protocol":{{"minVersion":"1.0","maxVersion":"1.0"}}}}}}"#).unwrap();
    stream.flush().unwrap();
    thread::sleep(Duration::from_millis(350));
    assert!(data_dir.join("run/instance.json").is_file());
    drop(stream);

    wait_for_absence(&data_dir.join("run/instance.json"), Duration::from_secs(2));
    assert_clean_shutdown(&data_dir);
    cleanup(&root, &data_dir);
}

fn spawn_transient(data_dir: &Path, idle_timeout_ms: u64) -> Child {
    Command::new(env!("CARGO_BIN_EXE_contexthub-app-server"))
        .args([
            "serve",
            "--transport",
            "local-ipc",
            "--transient",
            "--idle-timeout-ms",
            &idle_timeout_ms.to_string(),
            "--environment",
            "test",
            "--data-dir",
        ])
        .arg(data_dir)
        .arg("--runtime-dir")
        .arg(data_dir.join("run"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("transient App Server starts")
}

/// 等待瞬时实例把本地端点发布出来。就绪信号取实例记录文件而不是套接字文件：Windows
/// 命名管道不落地为文件，实例记录才是两端都会写下的发布标志；也不用连通性探测，探测会
/// 真的建立一次连接，对随后要断言的空闲计时有干扰。
fn wait_for_endpoint(data_dir: &Path, child: &mut Child) -> Endpoint {
    let runtime_dir = data_dir.join("run");
    let record_path = runtime_dir.join("instance.json");
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if record_path.exists() {
            return Endpoint::resolve(&runtime_dir).expect("endpoint resolves once published");
        }
        if let Some(status) = child.try_wait().expect("inspect App Server") {
            panic!("App Server exited before publishing IPC endpoint: {status}");
        }
        thread::sleep(Duration::from_millis(5));
    }
    child
        .kill()
        .expect("terminate App Server that missed readiness");
    child.wait().expect("reap App Server that missed readiness");
    let mut stderr = String::new();
    child
        .stderr
        .as_mut()
        .expect("stderr available")
        .read_to_string(&mut stderr)
        .expect("read App Server stderr");
    panic!("App Server did not publish IPC endpoint: {stderr}");
}

/// 实例记录先于端点发布落盘，所以拿到端点后仍需重试连接，直到端点真正可用。连上的这条
/// 连接直接交给调用方使用：瞬时实例把每次连接都计入活跃度，探测用的连接建了又断会干扰
/// 随后要断言的空闲退出计时。
fn connect_when_ready(endpoint: &Endpoint, timeout: Duration) -> LocalStream {
    let deadline = Instant::now() + timeout;
    loop {
        match LocalStream::connect(endpoint) {
            Ok(stream) => return stream,
            Err(error) => {
                assert!(
                    Instant::now() < deadline,
                    "client cannot connect to local endpoint: {error}"
                );
                thread::sleep(Duration::from_millis(5));
            }
        }
    }
}

#[cfg(unix)]
fn wait_for_path(path: &Path, timeout: Duration) -> PathBuf {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if path.exists() {
            return path.to_owned();
        }
        thread::sleep(Duration::from_millis(5));
    }
    panic!("expected path was not created: {}", path.display());
}

#[cfg(unix)]
fn wait_for_absence(path: &Path, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if !path.exists() {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!("expected path was not removed: {}", path.display());
}

fn wait_for_successful_exit(child: &mut Child, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait().expect("inspect App Server") {
            assert!(status.success(), "App Server exited with {status}");
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    child.kill().expect("terminate hung test server");
    child.wait().expect("reap hung test server");
    panic!("App Server did not exit after becoming idle");
}

fn assert_clean_shutdown(data_dir: &Path) {
    // 端点既不该留下文件系统残留，也不该继续接受连接；Windows 命名管道本就不落地为文件，
    // 残留判定在该端恒为假，连通性探测才是两端都有信号的断言（此时实例已退出，探测无副作用）。
    let endpoint = Endpoint::resolve(&data_dir.join("run")).unwrap();
    assert!(!artifact_exists(&endpoint).unwrap());
    assert!(!probe_liveness(&endpoint).unwrap());
    assert!(!data_dir.join("run/instance.json").exists());
    assert!(data_dir.join("run/instance.lock").is_file());
    assert!(data_dir.join("contexthub.db").is_file());
    assert!(!data_dir.join("contexthub.db-wal").exists());
    assert!(!data_dir.join("contexthub.db-shm").exists());
}

fn cleanup(root: &Path, data_dir: &Path) {
    fs::remove_file(data_dir.join("contexthub.db")).unwrap();
    fs::remove_file(data_dir.join("run/instance.lock")).unwrap();
    fs::remove_dir(data_dir.join("run")).unwrap();
    fs::remove_dir(data_dir).unwrap();
    fs::remove_dir(root).unwrap();
}

fn unique_root(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    // Unix 域套接字路径有约 104 字符上限，而 macOS 的 $TMPDIR 是很长的 /var/folders/…，
    // 套上用例名和进程号后会直接超限、导致端点解析失败。因此优先落在短路径 /tmp 上，
    // 只有在没有该目录的平台（Windows）才回退到系统临时目录。
    let base = if Path::new("/tmp").is_dir() {
        PathBuf::from("/tmp")
    } else {
        std::env::temp_dir()
    };
    base.join(format!(
        "ch-transient-{label}-{}-{nanos}",
        std::process::id()
    ))
}

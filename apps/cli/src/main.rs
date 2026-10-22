use contexthub_local_transport::{Endpoint, EndpointIdentity, LocalStream};
use contexthub_platform_adapter::instance::{read_descriptor, validate_descriptor};
use contexthub_protocol::{InitializeParams, PROTOCOL_VERSION};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn main() {
    if let Err(message) = run() {
        eprintln!("{message}");
        std::process::exit(10);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [command, server] if command == "handshake" => handshake(server),
        [command, endpoint] if command == "attach" => attach(endpoint),
        [command, server, environment, data_dir] if command == "ping" => {
            ping(server, environment, data_dir)
        }
        _ => Err(
            "usage: contexthub-cli handshake <app-server-path> | contexthub-cli attach <ipc-endpoint> | contexthub-cli ping <app-server-path> <environment> <data-dir>"
                .to_owned(),
        ),
    }
}

fn handshake(server: &str) -> Result<(), String> {
    let mut child = Command::new(server)
        .args(["serve", "--transport", "stdio"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|error| format!("cannot start App Server: {error}"))?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or("App Server stdin is unavailable")?;
    let stdout = child
        .stdout
        .take()
        .ok_or("App Server stdout is unavailable")?;
    let request = initialize_request();
    writeln!(stdin, "{request}").map_err(|error| error.to_string())?;
    stdin.flush().map_err(|error| error.to_string())?;
    let response = read_response(BufReader::new(stdout))?;
    validate_initialize(&request, &response, false)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&response["result"]).map_err(|error| error.to_string())?
    );
    drop(stdin);
    let status = child.wait().map_err(|error| error.to_string())?;
    if !status.success() {
        return Err(format!("App Server exited with {status}"));
    }
    Ok(())
}

fn attach(endpoint: &str) -> Result<(), String> {
    let identity = EndpointIdentity::parse(endpoint)
        .map_err(|error| format!("invalid local IPC endpoint: {error}"))?;
    let stream = LocalStream::connect_identity(&identity)
        .map_err(|error| format!("cannot connect to local App Server: {error}"))?;
    ping_stream(stream)
}

fn ping(server: &str, environment: &str, data_dir: &str) -> Result<(), String> {
    let data_dir = Path::new(data_dir);
    let runtime_dir = data_dir.join("run");
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut launched = None;
    loop {
        match discover_stream(data_dir, environment)? {
            Discovery::Connected(stream) => return ping_stream(stream),
            Discovery::Starting | Discovery::Missing | Discovery::Stale => {}
        }
        if launched.is_none() {
            launched = Some(
                Command::new(server)
                    .args([
                        "serve",
                        "--transport",
                        "local-ipc",
                        "--transient",
                        "--idle-timeout-ms",
                        "30000",
                        "--environment",
                        environment,
                        "--data-dir",
                    ])
                    .arg(data_dir)
                    .arg("--runtime-dir")
                    .arg(&runtime_dir)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .map_err(|error| format!("cannot start transient App Server: {error}"))?,
            );
        }
        if let Some(child) = launched.as_mut()
            && let Some(status) = child
                .try_wait()
                .map_err(|error| format!("cannot inspect transient App Server: {error}"))?
            && status.code() != Some(11)
        {
            return Err(format!(
                "transient App Server exited before becoming ready: {status}"
            ));
        }
        if Instant::now() >= deadline {
            return Err("transient App Server did not become ready within 5 seconds".to_owned());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

enum Discovery {
    Missing,
    Stale,
    Starting,
    Connected(LocalStream),
}

fn discover_stream(data_dir: &Path, environment: &str) -> Result<Discovery, String> {
    let record_path = data_dir.join("run/instance.json");
    let Some(descriptor) = read_descriptor(&record_path)
        .map_err(|error| format!("cannot read App Server instance record: {error}"))?
    else {
        return Ok(Discovery::Missing);
    };
    let runtime_dir = data_dir.join("run");
    let endpoint = Endpoint::resolve(&runtime_dir)
        .map_err(|error| format!("cannot resolve local IPC endpoint: {error}"))?;
    match validate_descriptor(&descriptor, data_dir, environment, &endpoint) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Discovery::Stale);
        }
        Err(error) => return Err(format!("invalid App Server instance record: {error}")),
    }
    let identity = EndpointIdentity::parse(&descriptor.ipc_endpoint)
        .map_err(|error| format!("invalid App Server IPC endpoint: {error}"))?;
    match LocalStream::connect_identity(&identity) {
        Ok(stream) => Ok(Discovery::Connected(stream)),
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
            ) =>
        {
            Ok(Discovery::Starting)
        }
        Err(error) => Err(format!("cannot connect to local App Server: {error}")),
    }
}

fn ping_stream(mut stream: LocalStream) -> Result<(), String> {
    let reader_stream = stream
        .try_clone()
        .map_err(|error| format!("cannot clone local IPC stream: {error}"))?;
    let mut reader = BufReader::new(reader_stream);
    let request = initialize_request();
    writeln!(stream, "{request}").map_err(|error| error.to_string())?;
    stream.flush().map_err(|error| error.to_string())?;
    let initialize = read_response(&mut reader)?;
    validate_initialize(&request, &initialize, true)?;

    writeln!(
        stream,
        "{}",
        json!({"jsonrpc": "2.0", "method": "initialized", "params": {}})
    )
    .map_err(|error| error.to_string())?;
    writeln!(
        stream,
        "{}",
        json!({"jsonrpc": "2.0", "id": "cli_ping_1", "method": "ping", "params": {}})
    )
    .map_err(|error| error.to_string())?;
    stream.flush().map_err(|error| error.to_string())?;
    let ping = read_response(&mut reader)?;
    if ping.get("error").is_some() {
        return Err(format!("ping failed: {ping}"));
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "initialize": initialize["result"],
            "ping": ping["result"]
        }))
        .map_err(|error| error.to_string())?
    );
    Ok(())
}

fn initialize_request() -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": "cli_initialize_1",
        "method": "initialize",
        "params": {
            "client": { "name": "contexthub-cli", "version": env!("CARGO_PKG_VERSION"), "platform": std::env::consts::OS, "arch": std::env::consts::ARCH },
            "protocol": { "minVersion": PROTOCOL_VERSION, "maxVersion": PROTOCOL_VERSION },
            "capabilities": { "interactive": false, "notifications": false, "resourceTickets": true, "eventReplay": true },
            "resume": {}
        }
    })
}

fn read_response<R: BufRead>(mut reader: R) -> Result<Value, String> {
    let mut line = String::new();
    let count = reader
        .read_line(&mut line)
        .map_err(|error| error.to_string())?;
    if count == 0 {
        return Err("App Server closed the connection before responding".to_owned());
    }
    serde_json::from_str(&line).map_err(|_| "App Server output is not valid JSONL".to_owned())
}

fn validate_initialize(
    request: &Value,
    response: &Value,
    require_local_ipc: bool,
) -> Result<(), String> {
    if response.get("error").is_some() {
        return Err(format!("initialize failed: {response}"));
    }
    if response["result"]["protocolVersion"] != PROTOCOL_VERSION {
        return Err("App Server negotiated an unexpected protocol version".to_owned());
    }
    serde_json::from_value::<InitializeParams>(request["params"].clone())
        .map_err(|error| error.to_string())?;
    if require_local_ipc && response["result"]["capabilities"]["localIpc"] != true {
        return Err("App Server did not confirm local IPC capability".to_owned());
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use contexthub_platform_adapter::instance::{
        INSTANCE_SCHEMA_VERSION, InstanceDescriptor, LifecycleMode, hash_canonical_path,
    };
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn dead_record_is_classified_for_guarded_stale_recovery() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            PathBuf::from("/tmp").join(format!("ch-cli-stale-{}-{nanos}", std::process::id()));
        let data_dir = root.join("test");
        let runtime_dir = data_dir.join("run");
        fs::create_dir_all(&runtime_dir).unwrap();
        let record = InstanceDescriptor {
            schema_version: INSTANCE_SCHEMA_VERSION,
            pid: std::process::id(),
            process_start_fingerprint: "not-the-current-start".to_owned(),
            server_instance_id: "stale".to_owned(),
            lifecycle_mode: LifecycleMode::Transient,
            environment: "test".to_owned(),
            data_dir_hash: hash_canonical_path(&data_dir).unwrap(),
            protocol_version: PROTOCOL_VERSION.to_owned(),
            ipc_endpoint: Endpoint::resolve(&runtime_dir)
                .unwrap()
                .identity()
                .into_owned(),
            started_at_epoch_ms: 0,
        };
        let record_path = runtime_dir.join("instance.json");
        fs::write(&record_path, serde_json::to_vec(&record).unwrap()).unwrap();
        fs::set_permissions(&record_path, fs::Permissions::from_mode(0o600)).unwrap();

        assert!(matches!(
            discover_stream(&data_dir, "test").unwrap(),
            Discovery::Stale
        ));

        fs::remove_file(record_path).unwrap();
        fs::remove_dir(runtime_dir).unwrap();
        fs::remove_dir(data_dir).unwrap();
        fs::remove_dir(root).unwrap();
    }
}

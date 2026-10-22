use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};

#[test]
fn stdio_is_jsonl_only_and_supports_full_lifecycle() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_contexthub-app-server"))
        .args(["serve", "--transport", "stdio"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("App Server starts");
    let mut stdin = child.stdin.take().expect("stdin available");
    let stdout = child.stdout.take().expect("stdout available");
    let mut reader = BufReader::new(stdout);

    writeln!(stdin, r#"{{"jsonrpc":"2.0","id":"init","method":"initialize","params":{{"client":{{"name":"contract-test","version":"0.1.0","platform":"macos","arch":"arm64"}},"protocol":{{"minVersion":"1.0","maxVersion":"1.0"}}}}}}"#).unwrap();
    stdin.flush().unwrap();
    let initialized = read_jsonl(&mut reader);
    assert_eq!(initialized["id"], "init");
    assert_eq!(
        initialized["result"]["limits"]["maxMessageBytes"],
        64 * 1024 * 1024
    );

    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","method":"initialized","params":{{}}}}"#
    )
    .unwrap();
    writeln!(stdin, r#"{{"jsonrpc":"2.0","id":2,"method":"ping"}}"#).unwrap();
    stdin.flush().unwrap();
    let ping = read_jsonl(&mut reader);
    assert_eq!(ping["id"], 2);
    assert_eq!(ping["result"]["protocolVersion"], "1.0");

    writeln!(
        stdin,
        "{}",
        json!({
            "jsonrpc": "2.0",
            "id": "large-request",
            "method": "unknown/large",
            "params": { "content": "x".repeat(9 * 1024 * 1024) }
        })
    )
    .unwrap();
    stdin.flush().unwrap();
    let large = read_jsonl(&mut reader);
    assert_eq!(large["id"], "large-request");
    assert!(large.get("error").is_some());

    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":"prepare","method":"shutdown/prepare"}}"#
    )
    .unwrap();
    stdin.flush().unwrap();
    assert_eq!(read_jsonl(&mut reader)["result"]["ready"], true);
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":"execute","method":"shutdown/execute"}}"#
    )
    .unwrap();
    stdin.flush().unwrap();
    assert_eq!(read_jsonl(&mut reader)["result"]["accepted"], true);
    drop(stdin);

    let status = child.wait().expect("App Server exits");
    assert!(status.success());
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    assert!(stderr.is_empty(), "unexpected stderr: {stderr}");
}

fn read_jsonl(reader: &mut impl BufRead) -> Value {
    let mut line = String::new();
    let count = reader.read_line(&mut line).expect("read response");
    assert!(count > 0, "App Server closed stdout unexpectedly");
    assert!(line.ends_with('\n'));
    serde_json::from_str(&line).expect("stdout line must be one JSON value")
}

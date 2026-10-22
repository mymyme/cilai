//! 平台隔离启动器后的标准 MCP 管道，不连接 Codex 或共享协议邮箱。
use serde_json::Value;
use std::io::Write;
use std::process::ChildStdin;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::time::Instant;

pub(super) struct Transport {
    sender: SyncSender<Value>,
    receiver: Receiver<Result<Value, String>>,
}
impl Transport {
    pub(super) fn new(
        mut stdin: ChildStdin,
        receiver: Receiver<Result<Value, String>>,
    ) -> Result<Self, String> {
        let (sender, writes) = mpsc::sync_channel::<Value>(8);
        // A capability may stop reading stdin after discovery. A dedicated,
        // bounded writer keeps that pipe from blocking the host's deadline.
        std::thread::Builder::new()
            .name("personal-mcp-stdin".into())
            .spawn(move || {
                for value in writes {
                    if writeln!(stdin, "{value}")
                        .and_then(|()| stdin.flush())
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .map_err(|error| format!("个人能力输入无法启动：{error}"))?;
        Ok(Self { sender, receiver })
    }
    pub(super) fn send(&mut self, value: &Value) -> Result<(), String> {
        self.sender
            .try_send(value.clone())
            .map_err(|_| "个人能力输入已关闭或队列超过限制，结果未确认。".into())
    }
    pub(super) fn next(&self, deadline: Instant) -> Result<Value, String> {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or("个人能力执行超时，结果未确认，请检查后再操作。")?;
        self.receiver
            .recv_timeout(remaining)
            .map_err(|_| "个人能力连接中断或执行超时，结果未确认，请检查后再操作。".to_owned())?
    }
}

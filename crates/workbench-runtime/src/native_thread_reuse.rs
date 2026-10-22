//! 通过公开线程状态复用空闲线程；进程退出后才释放原生存储写入权。
use super::{CodexHarnessSkill, ProgressCallback, read_response};
use serde_json::{Value, json};
use std::fs::File;
use std::io::{BufRead, BufWriter};

#[derive(Clone)]
pub(super) struct NativeThread {
    pub fingerprint: String,
    pub response: Value,
    pub journal: crate::persistent_threads::ThreadJournal,
}

pub(super) fn fingerprint(params: &Value, tools: &[Value], skills: &[CodexHarnessSkill]) -> String {
    let mut config = params.clone();
    if let Some(config) = config.as_object_mut() {
        for key in [
            "threadId",
            "dynamicTools",
            "allowProviderModelFallback",
            "serviceName",
            "experimentalRawEvents",
        ] {
            config.remove(key);
        }
    }
    crate::persistent_threads::digest(&json!({"config":config,"tools":tools,
        "skills":skills.iter().map(|skill| (&skill.name,&skill.path)).collect::<Vec<_>>() }))
}

// 官方线程空闲即可保留；无法保留时由调用者终止进程并确认退出后释放写入权。
pub(super) fn settle(
    thread_id: &str,
    retain_allowed: bool,
    send: &mut impl FnMut(&Value) -> Result<(), String>,
    reader: &mut impl BufRead,
    log: &mut BufWriter<File>,
    progress: &ProgressCallback,
) -> Result<bool, String> {
    send(
        &json!({"method":"thread/read","id":97,"params":{"threadId":thread_id,"includeTurns":false}}),
    )?;
    let status = read_response(reader, log, 97, progress, "lifecycle")?;
    if retain_allowed
        && status
            .pointer("/result/thread/status/type")
            .and_then(Value::as_str)
            == Some("idle")
    {
        Ok(true)
    } else {
        Err("harness_runtime_retire_required".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn thread_reuse_rejects_configuration_and_tool_changes() {
        let start = json!({"model":"m","cwd":"workspace","developerInstructions":"rules","sandbox":"workspace-write","dynamicTools":[],"serviceName":"test","experimentalRawEvents":true});
        let mut resume = start.clone();
        resume["threadId"] = json!("native-thread");
        resume.as_object_mut().unwrap().remove("serviceName");
        assert_eq!(
            fingerprint(&start, &[], &[]),
            fingerprint(&resume, &[], &[])
        );
        for field in ["model", "cwd", "developerInstructions", "sandbox"] {
            let mut changed = resume.clone();
            changed[field] = json!("changed");
            assert_ne!(
                fingerprint(&start, &[], &[]),
                fingerprint(&changed, &[], &[])
            );
        }
        assert_ne!(
            fingerprint(&start, &[], &[]),
            fingerprint(&resume, &[json!({"name":"new-tool"})], &[])
        );
    }
}

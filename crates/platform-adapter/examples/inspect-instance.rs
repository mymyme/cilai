//! 只读启动诊断：复用正式实例校验，不启动服务、不修复权限、不读取业务数据。

use contexthub_local_transport::{Endpoint, EndpointIdentity, LocalStream};
use contexthub_platform_adapter::instance::{
    INSTANCE_SCHEMA_VERSION, hash_canonical_path, process_start_fingerprint, read_descriptor,
    validate_descriptor,
};
use serde_json::{Value, json};
use std::io;
use std::path::Path;

fn error_value(error: &io::Error) -> Value {
    json!({
        "ok": false,
        "kind": format!("{:?}", error.kind()),
        "osError": error.raw_os_error(),
        "message": error.to_string(),
    })
}

fn inspect(data_dir: &Path, environment: &str) -> Value {
    let mut report = json!({
        "diagnosticVersion": 2,
        "dataDirectory": data_dir,
        "requestedEnvironment": environment,
    });
    let descriptor = match read_descriptor(&data_dir.join("run/instance.json")) {
        Ok(Some(descriptor)) => {
            report["descriptorRead"] = json!({ "ok": true, "present": true });
            descriptor
        }
        Ok(None) => {
            report["descriptorRead"] = json!({ "ok": true, "present": false });
            return report;
        }
        Err(error) => {
            report["descriptorRead"] = error_value(&error);
            return report;
        }
    };
    // 只输出实例元数据，不读取或连接业务数据库、会话与凭据。
    report["record"] = json!({
        "schemaVersion": descriptor.schema_version,
        "environment": descriptor.environment,
        "pid": descriptor.pid,
        "processStartFingerprint": descriptor.process_start_fingerprint,
        "dataDirHash": descriptor.data_dir_hash,
        "ipcEndpoint": descriptor.ipc_endpoint,
        "protocolVersion": descriptor.protocol_version,
    });
    report["schemaMatches"] = json!(descriptor.schema_version == INSTANCE_SCHEMA_VERSION);
    report["environmentMatches"] = json!(descriptor.environment == environment);
    match hash_canonical_path(data_dir) {
        Ok(hash) => {
            report["dataDirHashMatches"] = json!(descriptor.data_dir_hash == hash);
            report["expectedDataDirHash"] = json!(hash);
        }
        Err(error) => report["dataDirectoryCheck"] = error_value(&error),
    }
    match process_start_fingerprint(descriptor.pid) {
        Ok(fingerprint) => {
            report["processCheck"] = json!({
                "ok": true,
                "live": fingerprint.is_some(),
                "fingerprintMatches": fingerprint.as_ref()
                    == Some(&descriptor.process_start_fingerprint),
                "actualFingerprint": fingerprint,
            });
        }
        Err(error) => report["processCheck"] = error_value(&error),
    }
    let endpoint = match Endpoint::resolve(&data_dir.join("run")) {
        Ok(endpoint) => endpoint,
        Err(error) => {
            report["endpointCheck"] = error_value(&error);
            return report;
        }
    };
    report["expectedIpcEndpoint"] = json!(endpoint.identity());
    report["ipcEndpointMatches"] =
        json!(endpoint.matches_persisted_identity(&descriptor.ipc_endpoint));
    if let Err(error) = validate_descriptor(&descriptor, data_dir, environment, &endpoint) {
        report["descriptorValidation"] = error_value(&error);
        return report;
    }
    report["descriptorValidation"] = json!({ "ok": true });
    // 只有正式身份与进程校验通过才尝试连接；立即关闭，不发送任何协议或业务请求。
    // 不重试、不启动服务，失败仍保留原生错误，而不是推断实例已退出。
    let connection = EndpointIdentity::parse(&descriptor.ipc_endpoint)
        .and_then(|identity| LocalStream::connect_identity(&identity));
    report["ipcConnectionCheck"] = match connection {
        Ok(stream) => {
            drop(stream);
            json!({ "ok": true, "protocolRequestsSent": 0 })
        }
        Err(error) => error_value(&error),
    };
    report
}

fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 2 || !Path::new(&args[1]).is_absolute() {
        eprintln!("usage: inspect-instance <dev|prod> <absolute-profile-directory>");
        std::process::exit(2);
    }
    let Some(environment @ ("dev" | "prod")) = args[0].to_str() else {
        eprintln!("environment must be dev or prod");
        std::process::exit(2);
    };
    let report = inspect(Path::new(&args[1]), environment);
    match serde_json::to_string_pretty(&report) {
        Ok(output) => println!("{output}"),
        Err(error) => {
            eprintln!("cannot encode diagnostic report: {error}");
            std::process::exit(1);
        }
    }
}

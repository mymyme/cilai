//! One-time, public-interface migration from an existing legacy executable.
//! Native session files are copied opaquely; no Codex database, checksum, source,
//! lockfile or serialized session record is rewritten by Workbench.
use crate::codex_threads::CodexThreadService;
use contexthub_platform_adapter::security;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
    process::Command,
};

const RECEIPT: &str = "cilai-public-history-migration.json";
const EXPORT: &str = "cilai-legacy-public-export.json";
const MAX_EXPORT: u64 = 64 * 1024 * 1024;

fn verify_target(executable: &Path) -> Result<(), String> {
    let release: Value = serde_json::from_str(include_str!(
        "../../../third_party/openai-codex.release.json"
    ))
    .map_err(|_| "codex_upgrade_release_invalid")?;
    let mut source = fs::File::open(executable).map_err(|error| error.to_string())?;
    let mut digest = Sha256::new();
    let mut bytes = vec![0_u8; 64 * 1024];
    loop {
        let count = source.read(&mut bytes).map_err(|error| error.to_string())?;
        if count == 0 {
            break;
        }
        digest.update(&bytes[..count]);
    }
    let hash = format!("{:x}", digest.finalize());
    if !release["packages"]
        .as_object()
        .ok_or("codex_upgrade_release_invalid")?
        .values()
        .any(|package| package["executableSha256"] == hash)
    {
        return Err("codex_upgrade_official_binary_not_verified".into());
    }
    Ok(())
}

fn verified_receipt(home: &Path) -> Result<Value, String> {
    let path = home.join(RECEIPT);
    if fs::metadata(&path)
        .map_err(|error| error.to_string())?
        .len()
        > 64 * 1024
    {
        return Err("codex_upgrade_receipt_invalid".into());
    }
    let receipt = private_json(&path)?;
    let home = home.canonicalize().map_err(|error| error.to_string())?;
    if receipt["status"] != "verified"
        || receipt["targetVersion"] != "0.163.0"
        || receipt["homeDigest"] != crate::persistent_threads::digest(&json!(home))
    {
        return Err("codex_upgrade_verification_required".into());
    }
    Ok(receipt)
}

#[must_use]
pub fn official_home(legacy: &Path) -> PathBuf {
    legacy.join("official-runtime")
}

fn private_json(path: &Path) -> Result<Value, String> {
    security::verify_current_user_private_file(path)
        .map_err(|_| "codex_upgrade_private_file_invalid")?;
    let mut bytes = Vec::new();
    fs::File::open(path)
        .and_then(|file| file.take(MAX_EXPORT + 1).read_to_end(&mut bytes))
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_EXPORT {
        return Err("codex_upgrade_export_too_large".into());
    }
    serde_json::from_slice(&bytes).map_err(|_| "codex_upgrade_json_invalid".into())
}
fn write_new(path: &Path, value: &Value) -> Result<(), String> {
    let bytes = serde_json::to_vec(value).map_err(|error| error.to_string())?;
    if path.exists() {
        if private_json(path)? != *value {
            return Err("codex_upgrade_source_changed".into());
        }
        return Ok(());
    }
    let mut file = security::create_new_restricted_file(path).map_err(|error| error.to_string())?;
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| error.to_string())
}

/// New sessions use only the official home; existing opt-in migration receipts
/// must still belong to this Profile before their aliases or watermarks are used.
pub(crate) fn ensure_ready(ai: &Path) -> Result<(), String> {
    for name in ["cilai-harness", "cilai-application-harness"] {
        let legacy = ai.join(name);
        let home = official_home(&legacy);
        if home.join(RECEIPT).exists() {
            verified_receipt(&home)?;
        }
    }
    Ok(())
}

pub(crate) fn configure_aliases(command: &mut Command, home: &Path) -> Result<(), String> {
    let path = home.join(RECEIPT);
    if !path.exists() {
        return Ok(());
    }
    let receipt = verified_receipt(home)?;
    let aliases = receipt["providerAliases"]
        .as_array()
        .ok_or("codex_upgrade_provider_invalid")?;
    for alias in aliases {
        add_alias(
            command,
            alias.as_str().ok_or("codex_upgrade_provider_invalid")?,
        )?;
    }
    Ok(())
}
pub(crate) fn add_alias(command: &mut Command, alias: &str) -> Result<(), String> {
    if alias == "openai" {
        return Ok(());
    }
    if alias.is_empty()
        || alias.len() > 64
        || !alias
            .bytes()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == b'_')
    {
        return Err("codex_upgrade_provider_invalid".into());
    }
    // Metadata-only aliases let public thread/resume interpret historical providers.
    // Foreground configuration is appended afterwards and supplies the chosen model.
    // This closed local endpoint cannot reach the original provider or credentials.
    for (key, value) in [
        ("name", "\"Historical import\""),
        ("base_url", "\"http://127.0.0.1:1\""),
        ("wire_api", "\"responses\""),
    ] {
        command.args(["-c", &format!("model_providers.{alias}.{key}={value}")]);
    }
    Ok(())
}

fn export(service: &CodexThreadService) -> Result<Value, String> {
    let mut threads = Vec::new();
    let mut ids = BTreeSet::new();
    for archived in [false, true] {
        let mut cursor = Value::Null;
        let mut cursors = BTreeSet::new();
        loop {
            let page = service.request("thread/list", &json!({"archived":archived,"sourceKinds":[],"modelProviders":[],"useStateDbOnly":true,"limit":100,"cursor":cursor}))?;
            for thread in page["data"]
                .as_array()
                .ok_or("codex_upgrade_catalog_invalid")?
            {
                let id = thread["id"]
                    .as_str()
                    .ok_or("codex_upgrade_thread_invalid")?;
                if !ids.insert(id.to_owned()) || ids.len() > 10_000 {
                    return Err("codex_upgrade_catalog_invalid".into());
                }
                let metadata =
                    service.request("thread/read", &json!({"threadId":id,"includeTurns":false}))?;
                threads.push(json!({"thread":metadata["thread"],"archived":archived,"turns":service.turns(id)?}));
            }
            cursor = page
                .get("nextCursor")
                .cloned()
                .ok_or("codex_upgrade_catalog_invalid")?;
            if cursor.is_null() {
                break;
            }
            if !cursor.is_string() || !cursors.insert(cursor.to_string()) {
                return Err("codex_upgrade_cursor_invalid".into());
            }
        }
    }
    threads.sort_by(|a, b| a["thread"]["id"].as_str().cmp(&b["thread"]["id"].as_str()));
    let value = json!({"schemaVersion":1,"source":"legacy-public-app-server","threads":threads});
    if value.to_string().len() as u64 > MAX_EXPORT {
        return Err("codex_upgrade_export_too_large".into());
    }
    Ok(value)
}
fn clean(value: &Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.iter().map(clean).collect()),
        Value::Object(values) => Value::Object(
            values
                .iter()
                .filter(|(_, value)| !value.is_null())
                .map(|(key, value)| (key.clone(), clean(value)))
                .collect(),
        ),
        other => other.clone(),
    }
}
fn payload(turns: &[Value]) -> Value {
    clean(&Value::Array(turns.iter().map(|turn| json!({"id":turn["id"],"status":turn["status"],"error":turn["error"],"items":turn["items"],"startedAt":turn["startedAt"],"completedAt":turn["completedAt"],"durationMs":turn["durationMs"]})).collect()))
}
fn copy_rollout(thread: &Value, legacy: &Path, home: &Path) -> Result<(), String> {
    let path = Path::new(
        thread["path"]
            .as_str()
            .ok_or("codex_upgrade_rollout_missing")?,
    )
    .canonicalize()
    .map_err(|error| error.to_string())?;
    let relative = path
        .strip_prefix(legacy)
        .map_err(|_| "codex_upgrade_rollout_outside_home")?;
    if !["sessions", "archived_sessions"]
        .iter()
        .any(|name| relative.starts_with(name))
        || !path.is_file()
    {
        return Err("codex_upgrade_rollout_outside_home".into());
    }
    let target = home.join(relative);
    let parent = target.parent().ok_or("codex_upgrade_rollout_invalid")?;
    security::ensure_restricted_directory(parent).map_err(|error| error.to_string())?;
    if target.exists() {
        return Ok(());
    } // Pending receipt binds retries to the same export.
    let mut source = fs::File::open(path).map_err(|error| error.to_string())?;
    let mut target =
        security::create_new_restricted_file(&target).map_err(|error| error.to_string())?;
    std::io::copy(&mut source, &mut target)
        .and_then(|_| target.sync_all())
        .map_err(|error| error.to_string())
}

/// Migrate one existing native home using an existing legacy executable.
/// Both executables are supplied explicitly; the legacy executable is never bundled.
/// # Errors
/// Returns an explicit error on concurrent changes, partial/unknown history, or verification failure.
pub fn migrate(
    old_executable: &Path,
    official_executable: &Path,
    legacy: &Path,
) -> Result<Value, String> {
    verify_target(official_executable)?;
    let legacy = legacy.canonicalize().map_err(|error| error.to_string())?;
    let home = official_home(&legacy);
    security::ensure_restricted_directory(&home).map_err(|error| error.to_string())?;
    let _lock = security::try_lock_restricted_file(&home.join("cilai-migration.lock"))
        .map_err(|error| error.to_string())?
        .ok_or("codex_upgrade_already_running")?;
    let old = CodexThreadService::new(old_executable.to_path_buf(), legacy.clone());
    let source = export(&old)?;
    let digest = crate::persistent_threads::digest(&source);
    let pending = json!({"schemaVersion":1,"sourceDigest":digest,"targetVersion":"0.163.0"});
    write_new(&home.join("cilai-migration-pending.json"), &pending)?;
    write_new(&home.join(EXPORT), &source)?;
    let threads = source["threads"]
        .as_array()
        .ok_or("codex_upgrade_export_invalid")?;
    for thread in threads {
        copy_rollout(&thread["thread"], &legacy, &home)?;
    }
    let aliases = threads
        .iter()
        .filter_map(|value| value["thread"]["modelProvider"].as_str())
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    let target = CodexThreadService::new(official_executable.to_path_buf(), home.clone())
        .with_provider_aliases(aliases.iter().cloned().collect());
    let mut turns = 0;
    for before in threads {
        let id = before["thread"]["id"]
            .as_str()
            .ok_or("codex_upgrade_thread_invalid")?;
        if before["archived"] == true {
            target.request("thread/unarchive", &json!({"threadId":id}))?;
        }
        target.request("thread/resume", &json!({"threadId":id,"excludeTurns":true,"approvalPolicy":"never","sandbox":"read-only"}))?;
        if let Some(name) = before["thread"]["name"].as_str() {
            target.request("thread/name/set", &json!({"threadId":id,"name":name}))?;
        }
        let actual = target.turns(id)?;
        let expected = before["turns"]
            .as_array()
            .ok_or("codex_upgrade_export_invalid")?;
        if payload(&actual) != payload(expected) {
            return Err(format!("codex_upgrade_history_mismatch:{id}"));
        }
        let metadata =
            target.request("thread/read", &json!({"threadId":id,"includeTurns":false}))?;
        if metadata["thread"]["name"] != before["thread"]["name"]
            || metadata["thread"]["modelProvider"] != before["thread"]["modelProvider"]
        {
            return Err(format!("codex_upgrade_metadata_mismatch:{id}"));
        }
        if before["archived"] == true {
            target.request("thread/archive", &json!({"threadId":id}))?;
        }
        turns += actual.len();
    }
    target.shutdown();
    // Detect sends, edits, renames and archive changes in the old app during export.
    if crate::persistent_threads::digest(&export(&old)?) != digest {
        return Err("codex_upgrade_source_changed".into());
    }
    old.shutdown();
    let receipt = json!({"schemaVersion":1,"status":"verified","targetVersion":"0.163.0","homeDigest":crate::persistent_threads::digest(&json!(home.canonicalize().map_err(|error| error.to_string())?)),"sourceDigest":digest,"threads":threads.len(),"turns":turns,"providerAliases":aliases});
    write_new(&home.join(RECEIPT), &receipt)?;
    Ok(receipt)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_verification_checks_full_tool_payload_and_original_timestamps() {
        let before = vec![
            json!({"id":"t","status":"completed","startedAt":1,"completedAt":2,"durationMs":1000,"items":[{"id":"i","type":"mcpToolCall","result":{"text":"完整中文结果"}}]}),
        ];
        let mut after = before.clone();
        after[0]["items"][0]["newOptionalField"] = Value::Null;
        assert_eq!(payload(&before), payload(&after));
        after[0]["items"][0]["result"]["text"] = json!("被截断");
        assert_ne!(payload(&before), payload(&after));
        after = before.clone();
        after[0]["startedAt"] = json!(3);
        assert_ne!(payload(&before), payload(&after));
    }

    #[test]
    fn fresh_official_home_does_not_require_legacy_history_migration() {
        let root = std::env::temp_dir().join(format!("cilai-cutover-{}", uuid::Uuid::new_v4()));
        let legacy = root.join("cilai-harness");
        let home = official_home(&legacy);
        security::ensure_restricted_directory(&home).unwrap();
        let original = b"opaque original LF checksum database\n";
        fs::write(legacy.join("state_5.sqlite"), original).unwrap();
        ensure_ready(&root).unwrap();
        assert_eq!(fs::read(legacy.join("state_5.sqlite")).unwrap(), original);
        let mut receipt = json!({"status":"verified","targetVersion":"0.163.0","homeDigest":"another profile","providerAliases":[]});
        write_new(&home.join(RECEIPT), &receipt).unwrap();
        assert!(ensure_ready(&root).is_err());
        fs::remove_file(home.join(RECEIPT)).unwrap();
        receipt["homeDigest"] = json!(crate::persistent_threads::digest(&json!(
            home.canonicalize().unwrap()
        )));
        write_new(&home.join(RECEIPT), &receipt).unwrap();
        ensure_ready(&root).unwrap();
        assert_eq!(fs::read(legacy.join("state_5.sqlite")).unwrap(), original);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn provider_aliases_cannot_inject_configuration_or_credentials() {
        let mut command = Command::new("unused");
        for alias in [
            "",
            "a.b",
            "x\"\ncheck_for_update_on_startup=true",
            "../escape",
        ] {
            assert!(add_alias(&mut command, alias).is_err());
        }
        assert_eq!(command.get_args().count(), 0);
        add_alias(&mut command, "contexthub_qwen").unwrap();
        assert_eq!(command.get_args().count(), 6);
        assert!(command.get_envs().next().is_none());
    }
}

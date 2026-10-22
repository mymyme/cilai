use contexthub_platform_adapter::filesystem as guarded_fs;
use serde_json::{Value, json};
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

const MAX_READ_BYTES: u64 = 18_000;
const MAX_SEARCH_BYTES: u64 = 8 * 1024 * 1024;
const PAGE_SIZE: usize = 200;

pub(crate) fn is_report(request: &crate::codex_harness::CodexHarnessRequest) -> bool {
    request
        .application_mcp
        .as_ref()
        .is_some_and(|scope| scope.report_review)
}

pub(crate) fn configure_command(
    command: &mut Command,
    request: &crate::codex_harness::CodexHarnessRequest,
) -> Result<(), String> {
    if !is_report(request) {
        return Ok(());
    }
    if !request.skills.is_empty()
        || request.personal_tools.is_some()
        || request.application_adapter.is_none()
    {
        return Err("report_workspace_uncontrolled_tools".into());
    }
    validate_owned_skill_config(&request.harness_home)?;
    configure_project_config(command, &request.workspace_root)?;
    for setting in [
        "features.shell_tool=false",
        "features.view_image=false",
        "features.js_repl=false",
        "features.multi_agent=false",
        "features.code_mode=false",
        "features.mcp_resource_tools=false",
        "features.plugins=false",
        "features.skip_host_skill_discovery=true",
        "skills.bundled.enabled=false",
    ] {
        command.args(["-c", setting]);
    }
    Ok(())
}

fn configure_project_config(command: &mut Command, workspace: &Path) -> Result<(), String> {
    match fs::symlink_metadata(workspace.join(".codex/config.toml")) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        _ => return Err("report_workspace_uncontrolled_configuration".into()),
    }
    command.args(["-c", "project_root_markers=[]"]);
    Ok(())
}

fn validate_owned_skill_config(home: &Path) -> Result<(), String> {
    let path = home.join("config.toml");
    let mut file = match guarded_fs::open_regular_file_no_follow(&path, false) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err("report_workspace_uncontrolled_configuration".into()),
    };
    let mut content = String::new();
    (&mut file)
        .take(8193)
        .read_to_string(&mut content)
        .map_err(|_| "report_workspace_uncontrolled_configuration")?;
    let expected = format!(
        "[[skills.config]]\npath = {}\nenabled = false",
        json!(home.join("skills/.system/skill-creator/SKILL.md"))
    );
    if content.len() > 8192 || content.replace("\r\n", "\n").trim() != expected {
        return Err("report_workspace_uncontrolled_configuration".into());
    }
    Ok(())
}

pub(crate) fn validate_config(
    response: &Value,
    request: &crate::codex_harness::CodexHarnessRequest,
) -> Result<(), String> {
    let config = &response["result"]["config"];
    let controlled_endpoint = request
        .application_adapter
        .as_ref()
        .map(|(endpoint, _, _)| endpoint.as_str())
        .ok_or("report_workspace_uncontrolled_tools")?;
    let browser_endpoint = request
        .application_mcp
        .as_ref()
        .filter(|scope| !scope.browser_origins.is_empty())
        .and_then(|_| std::env::var("CILAI_BROWSER_MCP_URL").ok());
    validate_effective_config(
        config,
        controlled_endpoint,
        browser_endpoint.as_deref().map(str::trim),
    )
}

fn validate_effective_config(
    config: &Value,
    controlled_endpoint: &str,
    browser_endpoint: Option<&str>,
) -> Result<(), String> {
    if !config.is_object() {
        return Err("report_workspace_configuration_not_verified".into());
    }
    if config["project_root_markers"] != json!([]) {
        return Err("report_workspace_uncontrolled_configuration".into());
    }
    for feature in [
        "shell_tool",
        "view_image",
        "js_repl",
        "multi_agent",
        "code_mode",
        "plugins",
        "mcp_resource_tools",
    ] {
        if config["features"][feature] != false {
            return Err("report_workspace_uncontrolled_tools".into());
        }
    }
    if config["features"]["skip_host_skill_discovery"] != true
        || config["skills"]["bundled"]["enabled"] != false
    {
        return Err("report_workspace_uncontrolled_tools".into());
    }
    let servers = config["mcp_servers"]
        .as_object()
        .ok_or("report_workspace_configuration_not_verified")?;
    for (name, server) in servers {
        if server["enabled"] == false {
            continue;
        }
        let controlled =
            name == "cilai_application" && server["url"].as_str() == Some(controlled_endpoint);
        let browser = name == "cilai_browser"
            && browser_endpoint.is_some()
            && server["url"].as_str() == browser_endpoint;
        if !controlled && !browser {
            return Err("report_workspace_uncontrolled_mcp".into());
        }
    }
    for key in [
        "notify",
        "hooks",
        "model_instructions_file",
        "experimental_compact_prompt_file",
    ] {
        if !config[key].is_null() {
            return Err("report_workspace_uncontrolled_configuration".into());
        }
    }
    Ok(())
}

pub(crate) fn configure_adapter(
    adapter: &mut crate::application_tool_adapter::ApplicationToolAdapter,
    request: &crate::codex_harness::CodexHarnessRequest,
) -> Result<(), String> {
    if !is_report(request) {
        return Ok(());
    }
    let materials = Materials::new(&request.workspace_roots)?;
    adapter.add_report_materials(
        tool_definition(),
        Arc::new(move |args| materials.call(&args)),
    )
}

fn tool_definition() -> Value {
    json!({"title":"读取获准本地材料","description":"只读当前报告获准材料。roots列出范围；list分页列目录；read按UTF-8字节offset分页读取文本；search检索一个文本文件。禁止写入、命令执行与范围外路径，二进制文档须经获准文档处理工具读取。","inputSchema":{"type":"object","properties":{"operation":{"type":"string","enum":["roots","list","read","search"]},"path":{"type":"string"},"offset":{"type":"integer","minimum":0},"query":{"type":"string"}},"required":["operation"],"additionalProperties":false},"annotations":{"readOnlyHint":true,"destructiveHint":false}})
}

struct Grant {
    path: PathBuf,
    identity: String,
    directory: bool,
}
struct Materials {
    roots: Vec<Grant>,
}
impl Materials {
    fn new(roots: &[PathBuf]) -> Result<Self, String> {
        if roots.is_empty() || roots.iter().any(|path| !path.is_absolute()) {
            return Err("report_workspace_scope_invalid".into());
        }
        let roots = roots
            .iter()
            .map(|path| {
                let path = path
                    .canonicalize()
                    .map_err(|_| "report_workspace_material_unavailable")?;
                let snapshot = guarded_fs::metadata_with_stable_identity(&path)
                    .map_err(|_| "report_workspace_material_unavailable")?;
                Ok(Grant {
                    path,
                    identity: snapshot.stable_identity,
                    directory: snapshot.metadata.is_dir(),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(Self { roots })
    }

    fn resolve(&self, path: &str) -> Result<PathBuf, String> {
        let path = Path::new(path);
        if !path.is_absolute() {
            return Err("report_workspace_path_denied".into());
        }
        let target = path
            .canonicalize()
            .map_err(|_| "report_workspace_material_unavailable")?;
        if !self.roots.iter().any(|root| {
            root.path
                .canonicalize()
                .is_ok_and(|current| current == root.path)
                && guarded_fs::metadata_with_stable_identity(&root.path)
                    .is_ok_and(|current| current.stable_identity == root.identity)
                && (target == root.path || root.directory && target.starts_with(&root.path))
        }) {
            return Err("report_workspace_path_denied".into());
        }
        Ok(target)
    }

    fn open(&self, path: &Path) -> Result<(fs::File, String), String> {
        let expected = guarded_fs::symlink_metadata_with_stable_identity(path)
            .map_err(|_| "report_workspace_read_failed")?;
        let file = guarded_fs::open_regular_file_no_follow(path, false)
            .map_err(|_| "report_workspace_read_failed")?;
        self.verify_open(path, &file, &expected.stable_identity)?;
        Ok((file, expected.stable_identity))
    }

    fn verify_open(&self, path: &Path, file: &fs::File, expected: &str) -> Result<(), String> {
        let current = self.resolve(&path.to_string_lossy())?;
        let opened = guarded_fs::opened_file_metadata_with_stable_identity(file)
            .map_err(|_| "report_workspace_read_failed")?;
        let located = guarded_fs::symlink_metadata_with_stable_identity(path)
            .map_err(|_| "report_workspace_read_failed")?;
        if current != path
            || opened.stable_identity != expected
            || located.stable_identity != expected
        {
            return Err("report_workspace_material_changed".into());
        }
        Ok(())
    }

    fn call(&self, args: &Value) -> Result<Value, String> {
        let object = args
            .as_object()
            .ok_or("report_workspace_arguments_invalid")?;
        if object
            .keys()
            .any(|key| !["operation", "path", "offset", "query"].contains(&key.as_str()))
        {
            return Err("report_workspace_arguments_invalid".into());
        }
        let op = args["operation"]
            .as_str()
            .ok_or("report_workspace_arguments_invalid")?;
        if op == "roots" {
            if object.len() != 1 {
                return Err("report_workspace_arguments_invalid".into());
            }
            return Ok(
                json!({"roots":self.roots.iter().map(|grant| &grant.path).collect::<Vec<_>>()}),
            );
        }
        let path = self.resolve(
            args["path"]
                .as_str()
                .ok_or("report_workspace_arguments_invalid")?,
        )?;
        let offset = args.get("offset").map_or(Ok(0), |v| {
            v.as_u64().ok_or("report_workspace_offset_invalid")
        })?;
        match op {
            "list" => {
                let before = guarded_fs::metadata_with_stable_identity(&path)
                    .map_err(|_| "report_workspace_list_failed")?;
                let mut entries = fs::read_dir(&path)
                    .map_err(|_| "report_workspace_list_failed")?
                    .map(|entry| entry.map_err(|_| "report_workspace_list_failed"))
                    .take(25_001)
                    .collect::<Result<Vec<_>, _>>()?;
                if entries.len() > 25_000 {
                    return Err("report_workspace_directory_too_large".into());
                }
                entries.sort_by_key(std::fs::DirEntry::file_name);
                let start =
                    usize::try_from(offset).map_err(|_| "report_workspace_offset_invalid")?;
                if start > entries.len() {
                    return Err("report_workspace_offset_invalid".into());
                }
                let end = entries.len().min(start.saturating_add(PAGE_SIZE));
                let next = (end < entries.len()).then_some(end);
                let entries = entries[start..end].iter().map(|entry| {
                    let permitted = self.resolve(&entry.path().to_string_lossy()).is_ok();
                    json!({"path":entry.path(),"readable":permitted,"directory":permitted && entry.path().is_dir()})
                }).collect::<Vec<_>>();
                let after = guarded_fs::metadata_with_stable_identity(&path)
                    .map_err(|_| "report_workspace_list_failed")?;
                if before.stable_identity != after.stable_identity
                    || self.resolve(&path.to_string_lossy())? != path
                {
                    return Err("report_workspace_material_changed".into());
                }
                Ok(json!({"entries":entries,"nextOffset":next}))
            }
            "read" => self.read(&path, offset),
            "search" => {
                let query = args["query"]
                    .as_str()
                    .filter(|s| !s.is_empty() && s.len() <= 1000)
                    .ok_or("report_workspace_query_invalid")?;
                let content = self.text(&path, MAX_SEARCH_BYTES)?;
                let matches = content.lines().enumerate().filter(|(_, line)| line.contains(query)).map(|(line,text)| json!({"line":line+1,"text":text.chars().take(300).collect::<String>()})).collect::<Vec<_>>();
                let start =
                    usize::try_from(offset).map_err(|_| "report_workspace_offset_invalid")?;
                if start > matches.len() {
                    return Err("report_workspace_offset_invalid".into());
                }
                let end = matches.len().min(start.saturating_add(50));
                Ok(
                    json!({"matches":matches[start..end],"nextOffset":(end < matches.len()).then_some(end)}),
                )
            }
            _ => Err("report_workspace_operation_denied".into()),
        }
    }

    fn text(&self, path: &Path, limit: u64) -> Result<String, String> {
        let (mut file, identity) = self.open(path)?;
        if !file
            .metadata()
            .map_err(|_| "report_workspace_read_failed")?
            .is_file()
        {
            return Err("report_workspace_material_invalid".into());
        }
        let mut bytes = Vec::new();
        (&mut file)
            .take(limit + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "report_workspace_read_failed")?;
        self.verify_open(path, &file, &identity)?;
        if bytes.len() as u64 > limit {
            return Err("report_workspace_material_too_large".into());
        }
        if bytes.contains(&0) {
            return Err("report_workspace_text_required".into());
        }
        String::from_utf8(bytes).map_err(|_| "report_workspace_text_required".into())
    }

    fn read(&self, path: &Path, offset: u64) -> Result<Value, String> {
        let (mut file, identity) = self.open(path)?;
        let metadata = file
            .metadata()
            .map_err(|_| "report_workspace_read_failed")?;
        if !metadata.is_file() || offset > metadata.len() {
            return Err("report_workspace_offset_invalid".into());
        }
        file.seek(SeekFrom::Start(offset))
            .map_err(|_| "report_workspace_read_failed")?;
        let mut bytes = Vec::new();
        (&mut file)
            .take(MAX_READ_BYTES)
            .read_to_end(&mut bytes)
            .map_err(|_| "report_workspace_read_failed")?;
        self.verify_open(path, &file, &identity)?;
        if bytes.contains(&0) {
            return Err("report_workspace_text_required".into());
        }
        let content = match std::str::from_utf8(&bytes) {
            Ok(text) => text,
            Err(error)
                if error.error_len().is_none()
                    && offset + (bytes.len() as u64) < metadata.len() =>
            {
                std::str::from_utf8(&bytes[..error.valid_up_to()])
                    .map_err(|_| "report_workspace_text_required")?
            }
            Err(_) => return Err("report_workspace_text_required".into()),
        };
        let next = offset + content.len() as u64;
        Ok(
            json!({"path":path,"offset":offset,"text":content,"nextOffset":(next < metadata.len()).then_some(next),"totalBytes":metadata.len()}),
        )
    }
}

// 文件范围由受控材料工具逐次核验；官方 readOnly 只承担禁止写入的边界。
pub(crate) fn validate_thread(response: &Value, report_review: bool) -> Result<(), String> {
    if report_review
        && (response
            .pointer("/result/sandbox/type")
            .and_then(Value::as_str)
            != Some("readOnly")
            || response.pointer("/result/sandbox/networkAccess") != Some(&json!(false)))
    {
        return Err("report_workspace_policy_not_applied".into());
    }
    Ok(())
}

#[cfg(test)]
#[path = "report_workspace_tests.rs"]
mod tests;

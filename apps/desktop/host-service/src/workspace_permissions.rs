//! Desktop-only repair commands. Project paths come from Local App Server, never
//! from the renderer or a model. A single-use plan binds the displayed directory,
//! ACL, user, active profile and project until the user confirms it.
use contexthub_platform_adapter::{
    OperatingSystem, current_operating_system, security, workspace_permissions as native,
};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

struct Pending {
    profile: PathBuf,
    workspace_id: String,
    root: String,
    inspected: native::Inspection,
    created: Instant,
}
fn pending() -> &'static Mutex<HashMap<String, Pending>> {
    static PLANS: OnceLock<Mutex<HashMap<String, Pending>>> = OnceLock::new();
    PLANS.get_or_init(|| Mutex::new(HashMap::new()))
}
fn project_root(workspace_id: &str) -> Result<String, String> {
    let catalog = super::supervisor().request_result("workspace/list", &json!({}))?;
    select_project(&catalog, workspace_id)
}
fn select_project(catalog: &Value, workspace_id: &str) -> Result<String, String> {
    catalog
        .get("workspaces")
        .and_then(Value::as_array)
        .and_then(|items| {
            items.iter().find(|item| {
                item["workspaceId"].as_str() == Some(workspace_id)
                    && item["workspaceKind"].as_str() == Some("local")
            })
        })
        .and_then(|item| item["localRoot"].as_str())
        .map(str::to_owned)
        .ok_or_else(|| "仅能检查当前工作台中已登记的本地项目。请重新选择项目。".to_owned())
}
fn public_error(error: &std::io::Error) -> String {
    match error.raw_os_error() {
        Some(1223) => "已取消管理员授权，项目权限未修改。".to_owned(),
        Some(1237) => "目录或权限已变化，或确认已过期。请重新检查后再修复。".to_owned(),
        Some(5) => "Windows 拒绝修改此目录的权限。请让管理员检查目录所有权或企业策略，或改用你有权限的项目目录。".to_owned(),
        _ => match error.to_string().as_str() {
            "workspace_permission_reparse_point_forbidden" | "workspace_permission_local_directory_required" | "workspace_permission_drive_root_forbidden" | "workspace_permission_path_invalid" => "此目录不支持自动修复。请选择本地磁盘上的普通项目文件夹；不支持整盘、网络路径或目录链接。".to_owned(),
            "workspace_permission_result_unknown" => "等待修复结果超时。请稍后重新检查；当前不能确认是否已完成修复。".to_owned(),
            "workspace_permission_verification_failed" => "管理员操作已结束，但普通用户权限复查仍未通过。请让管理员检查目录权限。".to_owned(),
            _ => format!("项目权限检查或修复未完成，请重新检查或联系管理员。系统错误码：{}。", error.raw_os_error().map_or_else(|| "未提供".to_owned(), |code| code.to_string())),
        },
    }
}

pub(super) fn check(workspace_id: &str) -> Result<Value, String> {
    if current_operating_system() != OperatingSystem::Windows {
        return Ok(json!({"status":"notApplicable"}));
    }
    let profile = super::current_profile()?.data_dir;
    let root = project_root(workspace_id)?;
    let inspected =
        native::inspect(std::path::Path::new(&root)).map_err(|error| public_error(&error))?;
    let result = json!({"processElevated": inspected.process_elevated,"status": if inspected.can_update_permissions {"ready"} else {"repairRequired"},
        "directory": inspected.directory, "userSid": inspected.user_sid, "planId": null});
    if inspected.can_update_permissions {
        return Ok(result);
    }
    let mut plans = pending()
        .lock()
        .map_err(|_| "项目权限检查正在恢复，请重试。".to_owned())?;
    plans.retain(|_, plan| plan.created.elapsed() < Duration::from_mins(5));
    if plans.len() >= 32 {
        plans.clear();
    }
    let id = uuid::Uuid::new_v4().to_string();
    plans.insert(
        id.clone(),
        Pending {
            profile,
            workspace_id: workspace_id.to_owned(),
            root,
            inspected,
            created: Instant::now(),
        },
    );
    let mut result = result;
    result["planId"] = json!(id);
    Ok(result)
}

pub(super) fn repair(workspace_id: &str, plan_id: &str) -> Result<Value, String> {
    let plan = pending()
        .lock()
        .map_err(|_| "项目权限检查正在恢复，请重试。".to_owned())?
        .remove(plan_id)
        .ok_or_else(|| "修复确认已失效，请重新检查项目权限。".to_owned())?;
    if plan.created.elapsed() >= Duration::from_mins(5)
        || plan.profile != super::current_profile()?.data_dir
        || plan.workspace_id != workspace_id
        || project_root(workspace_id)? != plan.root
    {
        return Err("项目已切换或确认已过期，请重新检查项目权限。".to_owned());
    }
    let backup_directory = plan.profile.join("permission-repairs");
    security::ensure_restricted_directory(&backup_directory)
        .map_err(|error| public_error(&error))?;
    let backup = backup_directory.join(format!("{plan_id}.json"));
    let mut file =
        security::create_new_restricted_file(&backup).map_err(|error| public_error(&error))?;
    let record = json!({"schemaVersion":1, "workspaceId":workspace_id, "directory":plan.inspected.directory,
        "userSid":plan.inspected.user_sid, "originalSecurityDescriptor":plan.inspected.security_backup,
        "createdAt":chrono::Utc::now().to_rfc3339(), "scope":"directory-only WRITE_DAC"});
    file.write_all(
        serde_json::to_string_pretty(&record)
            .map_err(|error| error.to_string())?
            .as_bytes(),
    )
    .and_then(|()| file.sync_all())
    .map_err(|error| public_error(&error))?;
    let result = native::repair(&plan.inspected.plan).map_err(|error| public_error(&error))?;
    Ok(
        json!({"status":"ready", "directory": result.directory, "userSid":result.user_sid,
        "planId":null, "backupPath":backup.to_string_lossy()}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resolves_only_registered_local_projects() {
        let catalog = json!({"workspaces":[{"workspaceId":"a","workspaceKind":"local","localRoot":"C:\\项目"},
            {"workspaceId":"b","workspaceKind":"cloud_team","localRoot":"C:\\managed"}]});
        assert_eq!(select_project(&catalog, "a").unwrap(), "C:\\项目");
        assert!(select_project(&catalog, "b").is_err());
        assert!(select_project(&catalog, "C:\\Windows").is_err());
    }
    #[test]
    fn reports_cancel_stale_and_access_denied_without_claiming_success() {
        for (code, text) in [(1223, "取消"), (1237, "重新检查"), (5, "拒绝")] {
            assert!(public_error(&std::io::Error::from_raw_os_error(code)).contains(text));
        }
    }
}

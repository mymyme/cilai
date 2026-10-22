//! Recovery stays available while Local App Server is down. Paths and identity
//! come from this host, never from renderer input or model tools.
use contexthub_platform_adapter::{
    OperatingSystem, current_operating_system, startup_permissions as native,
};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

struct Pending {
    root: PathBuf,
    profile: String,
    plan: native::RepairPlan,
    created: Instant,
}
fn plans() -> &'static Mutex<HashMap<String, Pending>> {
    static VALUE: OnceLock<Mutex<HashMap<String, Pending>>> = OnceLock::new();
    VALUE.get_or_init(|| Mutex::new(HashMap::new()))
}
fn intended_profile() -> &'static Mutex<Option<String>> {
    static PROFILE: OnceLock<Mutex<Option<String>>> = OnceLock::new();
    PROFILE.get_or_init(|| Mutex::new(None))
}
pub(super) fn remember_profile(id: &str) {
    *intended_profile()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(id.to_owned());
}
fn profile_id() -> String {
    intended_profile()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
        .unwrap_or_else(|| {
            super::current_profile().map_or_else(
                |_| "unresolved-personal-profile".into(),
                |profile| profile.id,
            )
        })
}

fn public_error(error: &std::io::Error) -> String {
    if let Some((code, description)) = error.raw_os_error().and_then(native::conflict_reason) {
        return format!("startup_permission_repair_failed: {description}（错误类型：{code}）");
    }
    let detail = match error.raw_os_error() {
        Some(1223) => "已取消管理员授权。可重新检查后再次修复。",
        Some(1237) => "数据目录或账号已变化，或确认已过期。请重新检查后再修复。",
        Some(10001) => {
            "发现属于其他 Windows 账户的数据，已停止自动修复。请联系管理员核实数据归属。"
        }
        Some(10002) => "数据目录包含链接、硬链接或不支持的路径，已停止自动修复。",
        Some(32 | 33 | 10003) => "数据仍被另一个工作台或本地服务占用。请关闭其他工作台后重试。",
        Some(10004) => "权限复查未通过，尚不能确认启动已恢复。",
        Some(10005) => "修复失败，部分权限未能还原。原权限已备份，请导出错误日志联系管理员。",
        Some(5 | 1314 | 1300) => {
            "Windows 或组织策略拒绝了权限修复。请确认管理员授权可用，并导出错误日志反馈。"
        }
        _ if error.to_string().contains("result_unknown") => {
            "等待管理员修复结果超时，请稍后重新检查。当前不能确认是否修复完成。"
        }
        _ => "启动数据权限修复未完成，请导出错误日志反馈。",
    };
    format!(
        "startup_permission_repair_failed: {detail}（系统错误码：{}）",
        error
            .raw_os_error()
            .map_or_else(|| "未提供".into(), |code| code.to_string())
    )
}

pub(super) fn check() -> Result<Value, String> {
    if current_operating_system() != OperatingSystem::Windows {
        return Ok(json!({"status":"notApplicable"}));
    }
    let root = super::data_dir()?;
    let profile = profile_id();
    let inspection = native::inspect(&root, &profile).map_err(|error| public_error(&error))?;
    let id = uuid::Uuid::new_v4().to_string();
    let response = json!({"status":"confirmationRequired","planId":id,"directory":inspection.directory,"userSid":inspection.user_sid,"processElevated":inspection.process_elevated});
    let mut pending = plans()
        .lock()
        .map_err(|_| "启动修复正在恢复，请重试。".to_owned())?;
    pending.retain(|_, entry| entry.created.elapsed() < Duration::from_mins(5));
    if pending.len() >= 32 {
        pending.clear();
    }
    pending.insert(
        id,
        Pending {
            root,
            profile,
            plan: inspection.plan,
            created: Instant::now(),
        },
    );
    Ok(response)
}

pub(super) fn diagnostics() -> Value {
    if current_operating_system() != OperatingSystem::Windows {
        return json!({"status":"notApplicable"});
    }
    let result = super::data_dir()
        .map_err(std::io::Error::other)
        .and_then(|root| native::diagnose(&root, &profile_id()));
    match result {
        Ok(value) => json!({"status":"collected","context":value}),
        Err(error) => json!({"status":"unavailable","osError":error.raw_os_error()}),
    }
}

pub(super) fn repair(plan_id: &str) -> Result<Value, String> {
    let pending = plans()
        .lock()
        .map_err(|_| "启动修复正在恢复，请重试。".to_owned())?
        .remove(plan_id)
        .ok_or("修复确认已失效，请重新检查启动权限。")?;
    if pending.created.elapsed() >= Duration::from_mins(5)
        || pending.root != super::data_dir()?
        || pending.profile != profile_id()
    {
        return Err("账号或数据环境已变化，请重新检查启动权限。".into());
    }
    let status = super::supervisor().status();
    if status.connection_mode
        == Some(contexthub_workbench_runtime::supervisor::AppServerConnectionMode::Attached)
        || !super::ai_manager().tenant_switch_blockers().is_empty()
    {
        return Err("其他工作台或模型任务正在使用数据，请结束任务并关闭其他工作台后重试。".into());
    }
    super::ai_manager().reset_for_profile_switch()?;
    super::supervisor().shutdown_exclusive()?;
    *super::runtime_initialization_error()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    let result = recover(&pending, plan_id);
    if let Err(error) = &result
        && let Err(record_error) =
            super::supervisor().record_quiescent_failure("startup_permission_repair_failed", error)
    {
        eprintln!("startup_permission_repair_failed: {error}; state preservation: {record_error}");
    }
    result
}

fn recover(pending: &Pending, plan_id: &str) -> Result<Value, String> {
    let result = native::repair(&pending.plan, plan_id).map_err(|error| public_error(&error))?;
    // Always restart/recheck using this original Windows identity. A zero
    // exit code from an elevated child alone never means login has recovered.
    let profile = super::local_profile()?;
    super::set_active_profile(profile.clone())?;
    super::start_app_server(&profile.data_dir)?;
    super::initialize_ai_runtime(&profile.data_dir)?;
    Ok(json!({"status":"ready","backupPath":result.backup_path}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reports_cancel_conflict_busy_and_verification_failure() {
        for (code, text) in [
            (1223, "取消"),
            (1237, "变化"),
            (10001, "其他 Windows"),
            (10003, "占用"),
            (10004, "复查"),
            (10005, "备份"),
        ] {
            assert!(public_error(&std::io::Error::from_raw_os_error(code)).contains(text));
        }
        for code in 10011..=10018 {
            let (name, description) = native::conflict_reason(code).unwrap();
            let message = public_error(&std::io::Error::from_raw_os_error(code));
            assert!(message.contains(name));
            assert!(message.contains(description));
            assert!(!message.contains("系统错误码"));
        }
    }
    #[test]
    fn unissued_confirmation_cannot_trigger_native_repair() {
        assert!(repair("invented").unwrap_err().contains("失效"));
    }
}

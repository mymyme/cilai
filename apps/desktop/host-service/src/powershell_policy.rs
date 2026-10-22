//! Desktop-only confirmations; models cannot call these commands. Native plans
//! contain no renderer-controlled executable, registry path or shell command.
use contexthub_platform_adapter::{
    OperatingSystem, current_operating_system, powershell_policy as native, security,
};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

struct Pending {
    profile: PathBuf,
    plan: native::ChangePlan,
    states: Vec<native::EngineState>,
    restore: bool,
    created: Instant,
}
fn plans() -> &'static Mutex<HashMap<String, Pending>> {
    static VALUE: OnceLock<Mutex<HashMap<String, Pending>>> = OnceLock::new();
    VALUE.get_or_init(|| Mutex::new(HashMap::new()))
}
fn error(e: &std::io::Error) -> String {
    let detail = match e.raw_os_error() {
        Some(1223) => "已取消管理员授权。请重新检查后再试。",
        Some(1237) => "策略、PowerShell 安装或确认已变化，请重新检查。",
        Some(10005) => "策略修改失败，部分设置未能自动还原。已保留备份，请重新检查并恢复原策略。",
        _ => match e.to_string().as_str() {
            "powershell_policy_unsafe_installation" => {
                "PowerShell 安装目录可被普通用户修改或包含链接，不能自动修改系统策略。请让管理员检查安装。"
            }
            "powershell_policy_installation_busy" => {
                "PowerShell 安装文件正被其他程序占用，无法确认安全状态。请关闭相关程序后重新检查。"
            }
            "powershell_policy_managed" => "企业组策略已管理脚本执行，请联系管理员调整。",
            "powershell_policy_conflict" => {
                "策略或 PowerShell 安装已被其他操作修改，请重新检查，避免覆盖外部变更。"
            }
            "powershell_policy_verification_failed" => {
                "系统策略复查未通过，尚不能确认修改完成。请重新检查。"
            }
            "powershell_policy_query_timed_out" | "workspace_permission_result_unknown" => {
                "等待策略操作超时。请重新检查当前状态，暂不能确认结果。"
            }
            _ => "脚本执行策略检查或修改失败，请导出错误日志反馈。",
        },
    };
    format!("powershell_policy_failed: {detail}")
}
fn directory(profile: &Path) -> PathBuf {
    profile.join("powershell-policy")
}
fn recovery(profile: &Path) -> Result<Option<Vec<native::EngineState>>, String> {
    let file = directory(profile).join("recovery.json");
    if !file.try_exists().map_err(|e| error(&e))? {
        return Ok(None);
    }
    security::verify_current_user_private_file(&file).map_err(|e| error(&e))?;
    let mut text = String::new();
    std::fs::File::open(file)
        .map_err(|e| error(&e))?
        .take(16385)
        .read_to_string(&mut text)
        .map_err(|e| error(&e))?;
    if text.len() > 16384 {
        return Err("策略备份无法识别，请联系管理员检查。".into());
    }
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|_| "策略备份无法识别，请联系管理员检查。".into())
}
fn snapshot_same(a: &[native::EngineState], b: &[native::EngineState]) -> bool {
    a.len() == b.len()
        && a.iter().all(|a| {
            b.iter().any(|b| {
                a.engine == b.engine
                    && a.identity == b.identity
                    && a.scopes.local_machine == b.scopes.local_machine
            })
        })
}
fn register(
    profile: &Path,
    states: &[native::EngineState],
    plan: Option<native::ChangePlan>,
    restore: bool,
) -> Result<Option<String>, String> {
    let Some(plan) = plan else { return Ok(None) };
    let mut pending = plans()
        .lock()
        .map_err(|_| "策略检查正在恢复，请重试。".to_owned())?;
    pending.retain(|_, p| p.created.elapsed() < Duration::from_mins(5));
    if pending.len() >= 32 {
        pending.clear();
    }
    let id = uuid::Uuid::new_v4().to_string();
    pending.insert(
        id.clone(),
        Pending {
            profile: profile.to_owned(),
            plan,
            states: states.to_vec(),
            restore,
            created: Instant::now(),
        },
    );
    Ok(Some(id))
}
pub(super) fn check() -> Result<Value, String> {
    if current_operating_system() != OperatingSystem::Windows {
        return Ok(json!({"status":"notApplicable"}));
    }
    let profile = super::current_profile()?.data_dir;
    let states = native::inspect().map_err(|e| error(&e))?;
    let process_elevated = native::process_elevated().map_err(|e| error(&e))?;
    let original = recovery(&profile)?;
    let enable = if !states.iter().any(|s| s.scopes.managed())
        && original
            .as_ref()
            .is_none_or(|old| snapshot_same(old, &states))
    {
        native::prepare(&states, None).map_err(|e| error(&e))?
    } else {
        None
    };
    let mut restore_note = None;
    let restore = if let Some(old) = &original {
        match native::prepare(&states, Some(old)) {
            Ok(plan) => plan,
            Err(e) => {
                restore_note = Some(error(&e));
                None
            }
        }
    } else {
        None
    };
    let managed = states.iter().any(|s| s.scopes.managed());
    let phase = if managed {
        "managed"
    } else if states
        .iter()
        .any(|s| !s.scopes.local_machine.permits_local_scripts())
    {
        "restricted"
    } else {
        "configured"
    };
    let changes = enable
        .as_ref()
        .map(native::ChangePlan::preview)
        .unwrap_or_default();
    let restore_changes = restore
        .as_ref()
        .map(native::ChangePlan::preview)
        .unwrap_or_default();
    let plan_id = register(&profile, &states, enable, false)?;
    let restore_plan_id = register(&profile, &states, restore, true)?;
    Ok(
        json!({"status":phase,"processElevated":process_elevated,"engines":states,"planId":plan_id,"restorePlanId":restore_plan_id,"restoreNote":restore_note,"scope":"LocalMachine","changes":changes,"restoreChanges":restore_changes}),
    )
}
fn backup(pending: &Pending) -> Result<(), String> {
    if let Some(old) = recovery(&pending.profile)? {
        if !snapshot_same(&old, &pending.states) {
            return Err("已存在未恢复的策略备份，请先恢复原策略。".into());
        }
        return Ok(());
    }
    let root = directory(&pending.profile);
    security::ensure_restricted_directory(&root).map_err(|e| error(&e))?;
    let bytes = serde_json::to_vec_pretty(&pending.states).map_err(|e| e.to_string())?;
    // Keep an immutable audit copy even after successful in-app restoration.
    for path in [
        root.join(format!("backup-{}.json", uuid::Uuid::new_v4())),
        root.join("recovery.json"),
    ] {
        let mut file = security::create_new_restricted_file(&path).map_err(|e| error(&e))?;
        file.write_all(&bytes)
            .and_then(|()| file.sync_all())
            .map_err(|e| error(&e))?;
    }
    Ok(())
}
pub(super) fn apply(plan_id: &str) -> Result<Value, String> {
    let pending = plans()
        .lock()
        .map_err(|_| "策略检查正在恢复，请重试。".to_owned())?
        .remove(plan_id)
        .ok_or("策略确认已失效，请重新检查。")?;
    if pending.created.elapsed() >= Duration::from_mins(5)
        || pending.profile != super::current_profile()?.data_dir
    {
        return Err("账号已切换或确认已过期，请重新检查。".into());
    }
    if !super::ai_manager().tenant_switch_blockers().is_empty() {
        return Err("请等待正在执行的模型任务结束后再修改系统策略。".into());
    }
    if !pending.restore {
        backup(&pending)?;
    }
    native::apply(&pending.plan).map_err(|e| error(&e))?;
    if pending.restore {
        let path = directory(&pending.profile).join("recovery.json");
        security::verify_current_user_private_file(&path).map_err(|e| error(&e))?;
        std::fs::remove_file(path)
            .map_err(|_| "策略已恢复，但备份状态清理失败。请重新检查。".to_owned())?;
    }
    let mut result = check()?;
    result["result"] = json!(if pending.restore {
        "restored"
    } else {
        "updated"
    });
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invented_plan_never_triggers_elevation() {
        assert!(apply("not-issued").unwrap_err().contains("确认已失效"));
    }
    #[test]
    fn cancellation_and_conflicts_are_explicit() {
        assert!(error(&std::io::Error::from_raw_os_error(1223)).contains("取消"));
        assert!(error(&std::io::Error::from_raw_os_error(1237)).contains("重新检查"));
        assert!(error(&std::io::Error::from_raw_os_error(10005)).contains("备份"));
        assert!(
            error(&std::io::Error::other(
                "powershell_policy_installation_busy"
            ))
            .contains("其他程序占用")
        );
    }
}

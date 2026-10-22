use serde_json::json;
use std::path::Path;

pub(crate) async fn record(home: &Path, stage: &str, detail: &str) {
    let title = match stage {
        "failed_prepare_workspace" | "failed_empty_publication" | "failed_workspace_commit" => {
            "记忆目录初始化失败"
        }
        "failed_invalid_artifacts" => "记忆产物校验失败",
        "consolidation_timeout" => "记忆整理超时",
        "consolidation_token_budget" | "failed_resource_budget" => "记忆整理超出资源预算",
        "failed_publication" => "记忆发布失败",
        "failed_claim" | "failed_confirm_ownership" => "记忆任务状态异常",
        "failed_sandbox_policy" => "记忆整理权限配置无效",
        "failed_spawn_agent" | "failed_agent" => "记忆整理执行失败",
        _ => "记忆整理输入处理失败",
    };
    let stage_label = match stage {
        "failed_prepare_workspace" => "准备记忆目录",
        "failed_empty_publication" => "清理空记忆与重建基线",
        "failed_workspace_commit" => "保存记忆目录基线",
        "failed_invalid_artifacts" => "校验记忆索引与摘要",
        "failed_publication" => "发布记忆版本",
        "failed_claim" | "failed_confirm_ownership" => "确认记忆任务状态",
        "failed_resource_budget" | "consolidation_token_budget" => "检查整理资源预算",
        "failed_sources" | "failed_load_stage1_outputs" => "读取记忆来源",
        "failed_sync_workspace_inputs" | "failed_input_metadata" => "准备整理输入",
        "failed_workspace_status" | "failed_workspace_diff_file" => "检查记忆目录变更",
        "failed_sandbox_policy" => "检查整理权限配置",
        "failed_spawn_agent" => "启动整理任务",
        _ => "执行记忆整理",
    };
    // 上游已提供安全结构化诊断时保持原错误码、参数和请求编号。
    let payload = detail
        .find('{')
        .and_then(|start| serde_json::from_str::<serde_json::Value>(&detail[start..]).ok())
        .filter(|value| value["error"]["message"].is_string())
        .unwrap_or_else(|| {
            json!({"error": {
                "code": format!("MEMORY-{stage}"),
                "title": title,
                "message": detail,
                "diagnostics": {"stage": stage_label}
            }})
        });
    let error = format!("记忆整理失败：{payload}");
    if let Err(error) = crate::publication::status(home, "failed", Some(&error)).await {
        tracing::error!(%error, "failed persisting memory failure diagnostics");
    }
}

#[cfg(test)]
#[path = "failure_tests.rs"]
mod tests;

//! 桌面端可直接调用的应用服务方法白名单。
//!
//! 界面经一条通用命令把方法名与参数转交应用服务，方法名来自界面，因此必须在宿主侧
//! 逐项核准：缺了这道核准，任何能到达该命令的代码都能调用应用服务的全部方法，通用
//! 通道本身就成了绕过命令边界的入口。
//!
//! 单独成文件而不是留在各自的宿主里，是因为两个宿主壳都要用它。留在壳内就只能各带
//! 一份，而两份白名单发散不会以构建失败的方式暴露——它只会让一端悄悄比另一端宽松。

/// 判断界面是否可经通用通道调用该应用服务方法。
///
/// 采用正向列举：新方法必须显式加入才可调用，不在表内一律拒绝。
#[must_use]
pub fn desktop_method_allowed(method: &str) -> bool {
    matches!(
        method,
        "skill/list"
            | "skill/runList"
            | "skill/runRetry"
            | "skill/get"
            | "skill/save"
            | "skill/import"
            | "skill/importDirectory"
            | "skill/setState"
            | "skill/delete"
            | "skill/conversationCatalog"
            | "capabilityCreation/list"
            | "capabilityCreation/get"
            | "capabilityCreation/preview"
            | "personalApplication/open"
            | "personalCapability/export"
            | "workspace/list"
            | "workspace/create"
            | "file/hydrate"
            | "preview/readBinary"
            | "workspace/createDirectory"
            | "workspace/targetValidate"
            | "workspace/scan"
            | "workspace/files"
            | "workspace/listDirectory"
            | "workspace/indexRequest"
            | "workspace/indexStatus"
            | "workspace/indexPath"
            | "event/replay"
            | "project/list"
            | "project/create"
            | "project/attachWorkspace"
            | "project/detachWorkspace"
            | "workspace/delete"
            | "project/update"
            | "project/delete"
            | "project/get"
            | "projectPortfolio/list"
            | "deliverable/list"
            | "deliverable/create"
            | "deliverable/version/create"
            | "deliverable/version/review"
            | "myWork/get"
            | "globalSearch/query"
            | "unifiedSearch/query"
            | "projectProgress/get"
            | "phasePlan/create"
            | "phasePlan/activate"
            | "projectPhase/transition"
            | "businessTask/create"
            | "businessTask/get"
            | "businessTask/transition"
            | "businessTask/move"
            | "attentionCase/list"
            | "projectBlocker/create"
            | "decisionRequest/create"
            | "agentIncident/report"
            | "attentionCase/transition"
    )
}

#[cfg(test)]
mod tests {
    use super::desktop_method_allowed;

    #[test]
    fn desktop_rpc_allows_project_outcome_and_progress_methods() {
        for method in [
            "project/list",
            "workspace/createDirectory",
            "workspace/targetValidate",
            "project/create",
            "project/attachWorkspace",
            "project/detachWorkspace",
            "workspace/delete",
            "project/update",
            "project/delete",
            "project/get",
            "projectPortfolio/list",
            "deliverable/list",
            "deliverable/create",
            "deliverable/version/create",
            "deliverable/version/review",
            "myWork/get",
            "globalSearch/query",
            "unifiedSearch/query",
            "projectProgress/get",
            "phasePlan/create",
            "phasePlan/activate",
            "projectPhase/transition",
            "businessTask/create",
            "businessTask/get",
            "businessTask/transition",
            "businessTask/move",
            "attentionCase/list",
            "projectBlocker/create",
            "decisionRequest/create",
            "agentIncident/report",
            "attentionCase/transition",
            "skill/list",
            "skill/runList",
            "skill/runRetry",
            "skill/get",
            "skill/save",
            "skill/import",
            "skill/importDirectory",
            "skill/setState",
            "skill/delete",
            "skill/conversationCatalog",
            "capabilityCreation/list",
            "capabilityCreation/get",
            "capabilityCreation/preview",
            "personalApplication/open",
            "personalCapability/export",
        ] {
            assert!(desktop_method_allowed(method), "{method} should be allowed");
        }
    }

    #[test]
    fn desktop_rpc_allows_layered_browse_and_background_index_methods() {
        // 添加大文件夹时界面靠这四个方法逐层浏览、跟踪后台索引并优先打开单个文件；
        // 缺任何一个，界面都会在真实宿主里报 desktop_method_not_allowed。
        for method in [
            "workspace/listDirectory",
            "workspace/indexRequest",
            "workspace/indexStatus",
            "workspace/indexPath",
        ] {
            assert!(desktop_method_allowed(method), "{method} should be allowed");
        }
    }

    #[test]
    fn desktop_rpc_allows_file_preview_download_and_version_actions() {
        for method in ["file/hydrate", "preview/readBinary"] {
            assert!(
                desktop_method_allowed(method),
                "{method} should reach the authorized local service"
            );
        }
        assert!(!desktop_method_allowed("employeeCredential/get"));
        assert!(!desktop_method_allowed("file/readArbitraryPath"));
    }

    #[test]
    fn desktop_rpc_rejects_unknown_methods() {
        assert!(!desktop_method_allowed("system/executeArbitraryCommand"));
        assert!(!desktop_method_allowed("diagnostics/get"));
        assert!(!desktop_method_allowed("skill/runtimeCatalog"));
        assert!(!desktop_method_allowed("capabilityCreation/create"));
        for method in [
            "capabilityCreation/install",
            "applicationCatalog/list",
            "applicationCatalog/launch",
            "aiBrowserPolicy/get",
            "employeeCredential/get",
            "employeeCredential/save",
            "employeeCredential/delete",
        ] {
            assert!(!desktop_method_allowed(method), "{method}");
        }
    }

    #[test]
    fn desktop_rpc_keeps_ai_state_behind_the_runtime() {
        // 完整 AI 状态含本机路径与工具定义，写入会整体覆盖历史；界面只经脱敏的运行时快照读取
        for method in [
            "aiState/get",
            "aiState/replace",
            "aiState/apply",
            "aiTurn/finalize",
        ] {
            assert!(!desktop_method_allowed(method), "{method}");
        }
    }
}

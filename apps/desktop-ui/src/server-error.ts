/**
 * 把服务端的 reason 翻成一句人话。
 *
 * 服务端错误以 JSON-RPC 协议错误的形式回来，原样抛给用户就是一整串
 * `{"code":-32010,"data":{"reason":...}}`——里面确实有信息，但要用户自己去挑。
 * 这里只翻译那些"用户下一步能做点什么"的原因；翻不出来的保持原样，
 * 隐藏一个没预料到的错误比显示得难看更糟。
 */
const reasons: Record<string, string> = {
  cloud_download_destination_exists: "本地已有同名文件，请使用其他文件名。",
  cloud_download_cleanup_pending: "下载未完成，临时文件清理待完成。",
  cloud_download_cleanup_registration_failed: "无法登记临时文件清理，下载未开始。",
  cloud_download_profile_required: "本地数据目录尚未就绪，无法下载。",
  cloud_download_name_invalid: "文件名无效。",
  cloud_download_local_workspace_required: "请选择可用的本地空间。",
  cloud_material_version_changed: "云端文件已更新，请刷新后重试。",
  preview_source_changed: "文件已更新，请重新打开。",
  cloud_version_expired: "该历史版本已不在保留范围内。",
  cloud_version_policy_required: "管理员尚未配置版本保留策略。",
  cloud_version_limit_exceeded: "保留版本数量已达到上限。",
  cloud_file_worker_busy: "文件服务正忙，请稍后重试。",
  cloud_preview_unavailable: "文档预览服务尚未配置。",
  collaboration_permission_denied: "你已没有该文件的操作权限。",
  collaboration_resource_not_found: "文件或操作不存在，或已无权访问。",
  collaboration_operation_outcome_unknown: "结果尚未确认，请继续查询原操作。",
  file_hydration_timeout: "等待文件下载超时，可重新尝试。",
  file_hydration_download_failed: "系统未能完成文件下载。",
  file_hydration_source_changed: "文件在下载过程中发生变化，请重新打开。",
  file_hydration_provider_unsupported: "此文件提供方不支持自动下载。",
  cloud_workspace_sync_retired: "云端文件不再同步到本地，可主动下载为独立文件。",
  cloud_workspace_mirror_retired: "云端镜像同步已停用，请直接打开云端空间。",
  cloud_workspace_write_permission_required: "你没有该协作空间的编辑权限。",
  cloud_workspace_permission_revoked: "该协作空间已不可访问，请重新选择。",
  workspace_root_protected: "这是系统保护目录，不能作为工作文件夹。请选一个你自己的项目目录。",
  workspace_root_unavailable: "读不到这个文件夹。它可能已被移走、改名，或者还没有从云端下载下来。",
  workspace_root_not_directory: "选中的是一个文件，请选择文件夹。",
  workspace_root_non_unicode: "这个路径含有无法识别的字符，请换一个目录，或先给它改个名字。",
  workspace_root_environment_conflict: "这个文件夹属于另一个运行环境的数据目录，不能重复连接。",
  project_revision_conflict: "这个项目在你操作期间已被改动，请关闭后重新打开再试一次。",
  project_workspace_required: "请选择至少一个本地文件夹或云端空间。",
  application_catalog_request_failed: "企业应用服务暂时不可用，请稍后再试。",
  personal_capability_not_ready: "此应用尚未安装或已停用，请在应用中检查安装和启用状态。",
  personal_application_data_request_invalid: "应用数据请求无效，未切换版本。",
  personal_application_data_operation_unconfirmed: "应用数据操作尚未确认，请核对原记录，不要重复安装。",
  personal_application_data_restored_read_only: "恢复来源历史仅供查看，请在当前资料重新准备操作。",
  personal_application_data_source_changed: "数据在复制期间发生变化，原版本已保留。",
  personal_application_data_retained_changed: "保留的旧版数据已变化，未执行回退。",
  personal_application_data_snapshot_limit: "应用数据超过本次复制限制，原版本已保留。",
  personal_application_data_write_failed: "应用数据副本未能保存，原版本已保留。",
  personal_application_data_preview_changed: "版本或数据范围已变化，请重新核对。",
  personal_maintenance_release_unconfirmed: "维护结束尚未确认，请核对版本与数据记录。",
  skill_name_conflict: "已经有同名技能，请换一个名称。",
  skill_revision_conflict: "这个技能刚刚发生了变化，请重新打开后再试。",
  skill_not_found: "这个技能已不存在，请刷新列表。",
  skill_not_available: "这个技能当前不可用，请确认它仍处于启用状态。",
  skill_file_digest_mismatch: "技能文件与已保存版本不一致，已停止调用。请在技能页重新保存。",
  skill_file_path_invalid: "资源路径无效。请放在 scripts、references 或 assets 目录下。",
  skill_path_invalid: "生成的技能文件不在当前工作空间内，无法添加。",
  skill_file_invalid: "技能主文件内容无效，请检查名称、说明和正文。",
  skill_file_read_failed: "无法读取生成的技能文件，请检查文件是否仍在原位置。",
  skill_content_invalid: "技能内容无效，请检查名称、说明和正文。",
  skill_file_content_invalid: "资源文件过大或内容格式无效。",
  skill_files_invalid: "资源文件数量、总大小或路径存在冲突。",
  skill_directory_structure_invalid: "技能目录包含不支持的文件，请检查目录内容。",
  skill_files_read_only: "资源文件仅支持查看，不能在编辑技能时修改。",
  skill_archive_type_invalid: "请选择 ZIP 格式的技能包。",
  skill_archive_read_failed: "无法读取这个技能包，请确认文件仍在原位置且可访问。",
  skill_archive_invalid: "压缩包损坏、过大或包含不安全文件。",
  skill_archive_structure_invalid: "技能包目录结构无效。",
  skill_archive_content_invalid: "技能包中存在无法预览的非文本资源。",
  skill_archive_skill_file_missing: "压缩包根目录缺少 SKILL.md。",
  skill_package_sync_failed: "技能文件尚未安全保存，请检查存储后再试。",
  personal_package_import_response_invalid: "导入记录无法核对，操作已停止。请刷新记录。",
  personal_package_import_request_invalid: "导入请求无效，请重新选择安装包。",
  personal_package_import_target_invalid: "导入目标已失效，请重新选择。",
  personal_package_import_archive_invalid: "请选择有效的个人应用 ZIP 安装包。",
  personal_package_import_invalid: "安装包损坏或内容不受支持。",
  personal_package_import_manifest_invalid: "安装包声明与内容不一致，导入已停止。",
  personal_package_import_version_invalid: "来源版本格式不受支持。",
  personal_package_import_too_large: "安装包超过大小限制。",
  personal_package_import_archive_unavailable: "无法读取安装包，请重新选择。",
  personal_package_import_archive_path_invalid: "安装包路径无效，请重新选择。",
  personal_package_import_data_migration_unsupported: "此包声明了尚不支持的数据迁移，未安装。",
  personal_package_import_profile_not_ready: "本地资料尚未就绪，导入已停止。",
  personal_package_import_record_invalid: "导入记录校验未通过，请核对本地资料。",
  personal_package_import_not_found: "未找到此导入记录，请刷新。",
  personal_package_import_candidate_changed: "候选安装包已变化，请重新准备并确认。",
  personal_package_import_target_not_installed: "请先选择已安装的个人应用作为升级目标。",
  personal_package_import_target_changed: "升级目标已变化，请刷新目标应用后重新准备。",
  personal_package_import_sync_failed: "安装包尚未安全保存，导入保持未完成。",
  personal_package_import_write_failed: "无法保存候选安装包，请检查存储。",
  personal_package_import_destination_conflict: "候选目录存在冲突，原有内容已保留。",
  personal_package_import_preview_changed: "导入范围已变化，请重新核对。",
  personal_package_import_already_committed: "该导入已提交，请核对记录。",
  personal_package_import_cancelled: "该导入已取消，不能继续安装。",
  personal_package_import_permissions_unconfirmed: "权限尚未完整确认，未安装。",
  personal_package_import_list_refresh_failed: "导入已确认，应用列表读取失败，请单独刷新。",
};

/** 从协议错误里挑出 reason。挑不到就返回 null，由调用方决定怎么显示原文。 */
export function serverErrorReason(error: unknown): string | null {
  const text = String(error);
  const match = /"reason"\s*:\s*"([a-z0-9_]+)"/i.exec(text);
  return match ? match[1] : null;
}

/** 服务端错误的人话版本；没有对应说法时返回原文。 */
export function describeServerError(error: unknown): string {
  const reason = serverErrorReason(error) ?? (error instanceof Error ? error.message : typeof error === "string" ? error : null);
  return reason && Object.hasOwn(reasons, reason) ? reasons[reason] : String(error);
}

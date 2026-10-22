const test = require("node:test");
const assert = require("node:assert/strict");
const {personalCodes,diagnosticCategories}=require("./diagnostic-categories");
test("known connection failures retain distinct layers and never export arbitrary text",()=>{
  const text="model_selection_missing model_validation_network_failed model_auth_required model_validation_streaming_delta_missing model_validation_model_not_in_catalog model_validation_tool_denied model_runtime_permissions_audit_failed skill_package_sync_failed personal_package_import_cancelled token=PRIVATE_SECRET prompt=PRIVATE_TEXT model_unknown_PRIVATE";
  const codes=personalCodes(text);
  assert.deepEqual(diagnosticCategories(codes),["配置","网络","认证","协议","模型","工具","权限","保存","取消"]);
  assert.equal(JSON.stringify(codes).includes("PRIVATE"),false);
  assert.deepEqual(personalCodes("prefixmodel_auth_required model_auth_required_suffix"),[]);
});

test("MCP和资料维护失败保留实际分层且排除任意原文",()=>{
  const codes=personalCodes("mcp_credentials_required mcp_network_failed mcp_tool_outcome_unconfirmed personal_data_writer_active personal_data_export_failed secret=synthetic-private-value");
  assert.deepEqual(diagnosticCategories(codes),["认证","网络","工具","权限","保存"]);
  assert.equal(JSON.stringify(codes).includes("synthetic-private-value"),false);
});

test("宿主清理区分变更、执行身份与未确认回执，不导出未知扩展原文",()=>{
  const codes=personalCodes("personal_namespace_storage_segment_changed personal_namespace_storage_browser_transport_unconfirmed personal_namespace_action_unconfirmed personal_namespace_unknown_private_nonce token=private_value");
  assert.deepEqual(diagnosticCategories(codes),["配置","权限","保存"]);
  assert.equal(codes.length,3);
  assert.equal(JSON.stringify(codes).includes("private"),false);
});

test("页面失败区分版本授权动作与退出，页面正文不会进入导出",()=>{
  const codes=personalCodes("personal_page_binding_changed personal_page_scope_revoked personal_page_action_outcome_unconfirmed personal_page_close_unconfirmed prompt=PRIVATE_TEXT personal_page_unknown_private");
  assert.deepEqual(diagnosticCategories(codes),["配置","权限","工具","保存"]);
  assert.equal(codes.length,4);assert.equal(JSON.stringify(codes).includes("PRIVATE"),false);
});

test("本地凭据与旧引用清理保留安全分类",()=>{
  const codes=personalCodes("model_credential_vault_unavailable model_credential_reentry_required mcp_credential_reentry_required model_credential_legacy_cleanup_required mcp_credential_legacy_cleanup_required credential_vault_unknown_PRIVATE token=PRIVATE");
  assert.equal(codes.length,5); assert.deepEqual(diagnosticCategories(codes),["认证","保存"]); assert.equal(JSON.stringify(codes).includes("PRIVATE"),false);
});

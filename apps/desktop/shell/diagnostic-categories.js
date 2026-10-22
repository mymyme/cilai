const categories = {
  配置: ["model_connection_not_configured", "model_selection_missing", "model_connection_changed", "model_connection_disabled",
    "model_connection_revision_conflict", "model_endpoint_invalid", "model_endpoint_required", "model_provider_invalid",
    "model_purpose_binding_required", "model_purpose_binding_changed", "model_context_window_required", "model_validation_configuration_invalid"],
  网络: ["model_network_client_failed", "model_network_snapshot_changed", "model_network_snapshot_failed", "model_network_snapshot_unavailable",
    "model_catalog_network_failed", "model_validation_network_failed", "model_validation_network_http_rejected", "model_validation_timeout",
    "model_proxy_auth_not_supported", "model_proxy_scheme_not_supported", "model_proxy_url_invalid", "model_proxy_url_required",
    "model_system_proxy_read_failed", "model_system_proxy_invalid", "model_system_proxy_snapshot_invalid", "model_system_proxy_platform_not_supported",
    "model_system_proxy_pac_not_supported", "model_system_proxy_auto_discovery_not_supported", "model_system_proxy_scoped_not_supported",
    "model_system_proxy_bypass_not_supported", "model_system_proxy_socks_not_supported", "model_system_proxy_protocol_not_supported"],
  认证: ["AI-AUTH-001", "AI-AUTH-002", "AI-AUTH-003", "model_auth_required", "model_auth_login_unconfirmed", "model_auth_cancel_unconfirmed",
    "model_runtime_keyring_isolation_not_supported", "model_credential_reentry_required", "mcp_credential_reentry_required", "model_credential_vault_root_required", "model_credential_vault_unavailable",
    "model_auth_logout_unconfirmed", "model_auth_configuration_invalid", "model_auth_launch_failed", "model_auth_lifecycle_unconfirmed",
    "model_auth_process_alive", "model_auth_process_exit_unconfirmed", "model_auth_process_unconfirmed", "model_auth_state_unavailable",
    "model_credential_access_failed", "model_credential_invalid", "model_credential_required", "model_credential_write_failed",
    "model_validation_authentication_failed", "model_validation_authentication_not_established", "model_validation_authentication_response_invalid"],
  协议: ["AI-REQ-001", "AI-REQ-002", "AI-REQ-003", "response_envelope_invalid", "model_catalog_response_invalid",
    "model_execution_response_invalid", "model_validation_response_invalid", "model_validation_completion_missing",
    "model_validation_protocol_log_invalid", "model_validation_protocol_log_too_large", "model_validation_protocol_log_unavailable",
    "model_validation_runtime_version_mismatch", "model_validation_inference_nonce_mismatch", "model_validation_streaming_delta_missing",
    "codex_harness_empty_response"],
  模型: ["AI-MODEL-001", "AI-QUOTA-001", "AI-QUOTA-002", "AI-LIMIT-001", "AI-LIMIT-002", "model_catalog_access_failed",
    "model_validation_model_not_in_catalog", "model_validation_catalog_failed", "model_validation_runtime_failed"],
  工具: ["model_validation_tool_call_missing", "model_validation_tool_denied", "model_validation_tool_result_invalid", "model_validation_side_effect_observed",
    "model_runtime_mcp_not_authorized", "model_runtime_mcp_unconfirmed", "application_skill_not_installed", "application_skill_version_mismatch",
    "application_skill_snapshot_unavailable", "model_runtime_approved_skill_missing", "model_runtime_skill_path_invalid", "model_runtime_unapproved_skill",
    "model_runtime_skills_unconfirmed"],
  权限: ["model_history_consent_stale", "model_thread_history_consent_required", "model_runtime_auth_storage_not_isolated", "model_runtime_system_config_not_isolated",
    "model_runtime_home_isolation_changed", "model_runtime_home_isolation_gate_required", "model_runtime_home_isolation_unavailable",
    "model_runtime_isolation_full_access_not_supported", "model_runtime_file_permissions_invalid", "model_runtime_permissions_audit_failed",
    "model_runtime_tool_environment_not_isolated", "model_launch_gate_unavailable", "model_validation_workspace_failed",
    "workspace_acl_access_denied", "runtime_directory_owner_mismatch", "permission_verification_failed", "permission_check_failed", "process_access_denied"],
  保存: ["model_credential_legacy_cleanup_required", "mcp_credential_legacy_cleanup_required", "model_credential_operation_busy", "model_credential_cleanup_unconfirmed", "model_credential_delete_unconfirmed", "model_credential_readback_failed",
    "model_credential_save_unconfirmed", "skill_package_sync_failed", "personal_package_import_persistence_failed",
    "personal_package_import_candidate_invalid", "personal_package_import_target_changed", "local_profile_corrupt", "local_profile_changed",
    "local_profile_schema_unsupported", "model_runtime_home_failed", "model_runtime_environment_failed"],
  取消: ["model_validation_cleanup_unconfirmed", "model_execution_monitor_failed", "model_connection_in_use", "model_auth_cancelled",
    "personal_package_import_cancelled", "administrator_authorization_cancelled"],
};
categories.配置.push("mcp_configuration_invalid", "mcp_revision_changed", "mcp_catalog_changed", "mcp_validation_required", "personal_data_manifest_invalid", "personal_storage_preview_changed");
categories.网络.push("mcp_network_failed", "mcp_http_failed", "mcp_rate_limited", "mcp_local_timeout", "mcp_local_proxy_not_supported");
categories.认证.push("mcp_authentication_required", "mcp_credential_access_failed", "mcp_credentials_required", "mcp_credentials_invalid", "mcp_credentials_reentry_required", "mcp_credential_scope_invalid");
categories.协议.push("mcp_protocol_not_supported", "mcp_response_invalid", "mcp_response_id_mismatch", "mcp_response_contains_credentials", "personal_data_restore_result_unconfirmed");
categories.工具.push("mcp_tool_outcome_unconfirmed", "mcp_tool_not_authorized", "mcp_tool_authorization_revoked", "mcp_local_process_failed", "mcp_local_exit_unconfirmed", "mcp_stdio_write_unconfirmed");
categories.权限.push("mcp_local_isolation_not_supported", "mcp_child_processes_not_supported", "mcp_file_permissions_invalid", "mcp_file_permissions_changed", "mcp_original_home_permissions_protected", "mcp_source_changed", "mcp_managed_storage_protected", "mcp_managed_storage_invalid", "ai_memory_enable_consent_required", "ai_memory_enable_consent_stale", "personal_data_writer_active", "personal_maintenance_drain_unconfirmed", "personal_writer_process_unconfirmed", "personal_storage_item_protected");
categories.保存.push("personal_data_storage_failed", "personal_data_artifact_changed", "personal_data_export_failed", "personal_data_manifest_switch_unconfirmed", "personal_data_profile_files_unclassified", "personal_data_maintenance_release_unconfirmed", "personal_maintenance_release_unconfirmed", "personal_storage_delete_failed", "personal_storage_result_unconfirmed", "mcp_credential_write_failed", "mcp_credential_readback_failed", "mcp_credential_delete_unconfirmed");
categories.配置.push("personal_memory_reset_preview_changed", "personal_memory_reset_scope_changed", "personal_application_data_preview_changed", "personal_application_data_restored_read_only");
categories.协议.push("personal_memory_reset_response_unconfirmed", "personal_memory_reset_response_invalid");
categories.权限.push("personal_memory_reset_scope_denied", "personal_memory_reset_operation_unconfirmed", "personal_application_data_operation_unconfirmed");
categories.保存.push("personal_application_data_source_changed", "personal_application_data_retained_changed", "personal_application_data_snapshot_limit", "personal_application_data_write_failed", "personal_application_data_activation_unconfirmed", "personal_memory_reset_storage_failed");
categories.配置.push("personal_namespace_request_invalid", "personal_namespace_preview_changed", "personal_namespace_segment_invalid", "personal_namespace_storage_segment_changed", "personal_namespace_storage_segment_invalid");
categories.权限.push("personal_namespace_shell_owner_unconfirmed", "personal_namespace_execution_processes_unconfirmed", "personal_namespace_storage_typed_ack_required", "personal_namespace_storage_browser_transport_unconfirmed");
categories.保存.push("personal_namespace_action_unconfirmed", "personal_namespace_maintenance_release_unconfirmed", "personal_namespace_segment_sync_failed", "personal_namespace_storage_log_failed", "personal_namespace_storage_cache_failed");
categories.配置.push("personal_page_manifest_invalid", "personal_page_manifest_missing", "personal_page_binding_changed", "personal_page_revision_changed", "personal_page_context_expired");
categories.权限.push("personal_page_ai_not_authorized", "personal_page_scope_revoked", "personal_page_model_not_granted", "personal_page_confirmation_host_required", "personal_page_document_closed", "personal_page_frame_changed");
categories.工具.push("personal_page_action_outcome_unconfirmed", "personal_page_action_not_declared", "personal_page_tool_not_granted", "application_write_arguments_invalid");
categories.保存.push("personal_page_operation_unconfirmed", "personal_page_close_unconfirmed", "personal_page_retirement_unconfirmed");
const byCode = new Map(Object.entries(categories).flatMap(([category, codes]) => codes.map(code => [code, category])));
function personalCodes(text) {
  if (typeof text !== "string") return [];
  return [...new Set(text.match(/\b(?:AI-[A-Z]+-\d{3}|[a-z][a-z0-9]*(?:_[a-z0-9]+)+)\b/g) ?? [])].filter(code => byCode.has(code));
}
function diagnosticCategories(codes) {
  return [...new Set((Array.isArray(codes) ? codes : []).map(code => byCode.get(code)).filter(Boolean))];
}
module.exports = { personalCodes, diagnosticCategories, personalDiagnosticCodes: new Set(byCode.keys()) };

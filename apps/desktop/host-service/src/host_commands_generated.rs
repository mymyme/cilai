// Generated from apps/desktop/host-commands.json by scripts/generate-host-commands.mjs.

pub(super) fn is_parallel_command(value: &str) -> bool {
    matches!(
        value,
        "ai_automation_rule_run"
            | "ai_conversation_search"
            | "ai_conversation_start"
            | "ai_conversation_submit"
            | "ai_message_regenerate"
            | "ai_message_send"
            | "ai_message_submit"
            | "ai_model_catalog_read"
            | "ai_runtime_query"
            | "mcp_connection_validate"
            | "model_connection_auth_cancel"
            | "model_connection_auth_cancel_readonly"
            | "model_connection_auth_finish_logout"
            | "model_connection_auth_logout"
            | "model_connection_auth_read"
            | "model_connection_auth_resume"
            | "model_connection_auth_start"
            | "model_connection_catalog_refresh"
            | "model_connection_validate"
            | "model_execution_preflight"
            | "personal_application_data_rollback"
            | "personal_capability_upgrade"
            | "personal_data_backup_capture"
            | "personal_data_restore_apply"
            | "personal_memory_reset_confirm"
            | "personal_namespace_storage_action_ack_internal"
            | "personal_namespace_storage_action_begin_internal"
            | "personal_namespace_storage_execute_internal"
            | "personal_namespace_storage_finish"
            | "personal_namespace_storage_segment_sync_internal"
            | "personal_notification_preferences_get"
            | "personal_notification_preferences_save"
            | "personal_notifications_claim_internal"
            | "personal_notifications_open_internal"
            | "personal_notifications_read_internal"
            | "personal_notifications_report_internal"
            | "personal_package_import_commit"
            | "personal_page_ai_close_internal"
            | "personal_page_ai_dispatch_internal"
            | "personal_page_ai_register_internal"
            | "personal_page_ai_resolve_internal"
            | "personal_page_ai_revoke_all_internal"
            | "personal_storage_execute"
    )
}

pub(super) fn is_parallel_app_server_method(value: &str) -> bool {
    matches!(value, "workspace/listDirectory")
}

pub(super) fn requires_exclusive_data(value: &str) -> bool {
    matches!(
        value,
        "personal_application_data_rollback"
            | "personal_capability_upgrade"
            | "personal_data_backup_capture"
            | "personal_data_restore_apply"
            | "personal_memory_reset_confirm"
            | "personal_namespace_storage_execute_internal"
            | "personal_namespace_storage_finish"
            | "personal_package_import_commit"
            | "personal_storage_execute"
    )
}

pub(super) fn is_data_observation(value: &str) -> bool {
    matches!(
        value,
        "ai_automation_trigger_history"
            | "ai_runtime_query"
            | "app_server_status"
            | "host_platform"
            | "personal_namespace_storage_action_ack_internal"
            | "personal_namespace_storage_action_begin_internal"
            | "personal_namespace_storage_get"
            | "personal_namespace_storage_list"
            | "personal_namespace_storage_reconcile"
            | "personal_namespace_storage_segment_sync_internal"
            | "personal_page_ai_close_internal"
            | "personal_page_ai_dispatch_internal"
            | "personal_page_ai_register_internal"
            | "personal_page_ai_resolve_internal"
            | "personal_page_ai_revoke_all_internal"
            | "test_runtime_diagnostics"
    )
}

pub(super) fn changes_local_context(value: &str) -> bool {
    matches!(
        value,
        "app_server_restart"
            | "powershell_policy_apply"
            | "startup_permissions_check"
            | "startup_permissions_repair"
    )
}

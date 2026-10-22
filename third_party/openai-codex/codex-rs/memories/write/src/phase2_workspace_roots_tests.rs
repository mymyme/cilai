use super::agent;
use crate::memory_root;
use codex_model_provider::create_model_provider;
use codex_protocol::protocol::SandboxPolicy;
use core_test_support::responses::start_mock_server;
use core_test_support::test_codex::test_codex;
use pretty_assertions::assert_eq;
use std::sync::Arc;
use tempfile::TempDir;

#[tokio::test]
async fn consolidation_rebinds_workspace_roots_to_memory_root() -> anyhow::Result<()> {
    let server = start_mock_server().await;
    let home = Arc::new(TempDir::new()?);
    let test = test_codex()
        .with_home(home)
        .build_with_auto_env(&server)
        .await?;
    let provider = create_model_provider(
        test.config.model_provider.clone(),
        Some(test.thread_manager.auth_manager()),
    );

    let parent_permission_profile = test.config.permissions.effective_permission_profile();
    let agent_config =
        agent::get_config(&test.config, parent_permission_profile, provider.as_ref())
            .expect("agent config should be created");
    let root = memory_root(&test.config.codex_home);
    let worker_temp = root.join(".worker-tmp");

    assert_eq!(agent_config.cwd, root);
    assert_eq!(
        agent_config.workspace_roots,
        vec![root.clone(), worker_temp.clone()]
    );
    for name in ["TMPDIR", "TMP", "TEMP"] {
        assert_eq!(
            agent_config
                .permissions
                .shell_environment_policy
                .r#set
                .get(name),
            Some(&worker_temp.display().to_string())
        );
    }
    assert_eq!(
        agent_config.legacy_sandbox_policy(),
        SandboxPolicy::WorkspaceWrite {
            writable_roots: vec![worker_temp],
            network_access: false,
            exclude_tmpdir_env_var: true,
            exclude_slash_tmp: true,
        }
    );

    test.codex.shutdown_and_wait().await?;
    Ok(())
}

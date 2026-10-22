#![cfg(test)]

use super::*;
use std::fs;

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("report-read-scope-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        Self(root.canonicalize().unwrap())
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn report_retains_official_write_protection_while_material_tool_checks_read_scope() {
    let readonly = json!({"result":{"sandbox":{"type":"readOnly","networkAccess":false}}});
    validate_thread(&readonly, true).unwrap();
    for changed in [
        json!({"result":{}}),
        json!({"result":{"sandbox":{"type":"workspaceWrite","networkAccess":false}}}),
        json!({"result":{"sandbox":{"type":"dangerFullAccess"}}}),
        json!({"result":{"sandbox":{"type":"readOnly","networkAccess":true}}}),
    ] {
        assert_eq!(
            validate_thread(&changed, true).unwrap_err(),
            "report_workspace_policy_not_applied"
        );
    }
    validate_thread(&json!({"result":{}}), false).unwrap();
}

#[test]
fn report_accepts_only_the_owned_disabled_creator_config_across_runs() {
    let fixture = Fixture::new();
    validate_owned_skill_config(&fixture.0).unwrap();
    let disabled = format!(
        "[[skills.config]]\npath = {}\nenabled = false\n",
        json!(fixture.0.join("skills/.system/skill-creator/SKILL.md"))
    );
    fs::write(fixture.0.join("config.toml"), &disabled).unwrap();
    validate_owned_skill_config(&fixture.0).unwrap();
    validate_owned_skill_config(&fixture.0).unwrap();
    for changed in [
        disabled.replace("enabled = false", "enabled = true"),
        format!("{disabled}\n[mcp_servers.uncontrolled]\ncommand=\"reader\"\n"),
        disabled.replace("skill-creator", "other-skill"),
    ] {
        fs::write(fixture.0.join("config.toml"), changed).unwrap();
        assert!(validate_owned_skill_config(&fixture.0).is_err());
    }
}

#[test]
fn effective_report_configuration_rejects_inherited_servers_and_file_entry_points() {
    let endpoint = "http://127.0.0.1:1234/application-mcp";
    let safe = json!({"project_root_markers":[],"features":{"shell_tool":false,"view_image":false,"js_repl":false,"multi_agent":false,"code_mode":false,"plugins":false,"mcp_resource_tools":false,"skip_host_skill_discovery":true},"skills":{"bundled":{"enabled":false}},"mcp_servers":{"cilai_application":{"url":endpoint,"enabled":true}}});
    validate_effective_config(&safe, endpoint, None).unwrap();
    for markers in [Value::Null, json!([".git"]), json!("[]"), json!({})] {
        let mut changed = safe.clone();
        changed["project_root_markers"] = markers;
        assert_eq!(
            validate_effective_config(&changed, endpoint, None).unwrap_err(),
            "report_workspace_uncontrolled_configuration"
        );
    }
    let mut inherited = safe.clone();
    inherited["mcp_servers"]["uncontrolled"] = json!({"command":"arbitrary-reader"});
    assert_eq!(
        validate_effective_config(&inherited, endpoint, None).unwrap_err(),
        "report_workspace_uncontrolled_mcp"
    );
    let mut wrong_endpoint = safe.clone();
    wrong_endpoint["mcp_servers"]["cilai_application"]["url"] = json!("http://127.0.0.1:2/other");
    assert!(validate_effective_config(&wrong_endpoint, endpoint, None).is_err());
    for feature in [
        "shell_tool",
        "view_image",
        "js_repl",
        "multi_agent",
        "code_mode",
        "plugins",
    ] {
        let mut changed = safe.clone();
        changed["features"][feature] = json!(true);
        assert!(validate_effective_config(&changed, endpoint, None).is_err());
    }
    for key in [
        "notify",
        "hooks",
        "model_instructions_file",
        "experimental_compact_prompt_file",
    ] {
        let mut changed = safe.clone();
        changed[key] = json!("external-entry");
        assert!(validate_effective_config(&changed, endpoint, None).is_err());
    }
}

#[test]
fn report_project_config_is_isolated_from_home_ancestors_and_forces_the_current_directory() {
    let fixture = Fixture::new();
    let home = fixture.0.join("user-home");
    let workspace = home.join("Library/Application Support/profile/application-ai/policy");
    fs::create_dir_all(home.join(".codex")).unwrap();
    fs::create_dir_all(&workspace).unwrap();
    let user_config = home.join(".codex/config.toml");
    let contents = "model = \"user-owned-model\"\n";
    fs::write(&user_config, contents).unwrap();
    let mut command = Command::new("unused");
    configure_project_config(&mut command, &workspace).unwrap();
    assert_eq!(
        command.get_args().collect::<Vec<_>>(),
        ["-c", "project_root_markers=[]"]
    );
    assert_eq!(fs::read_to_string(user_config).unwrap(), contents);
    fs::create_dir(workspace.join(".codex")).unwrap();
    let project_config = workspace.join(".codex/config.toml");
    for directory in [false, true] {
        if directory {
            fs::remove_file(&project_config).unwrap();
            fs::create_dir(&project_config).unwrap();
        } else {
            fs::write(&project_config, "model = \"project-model\"\n").unwrap();
        }
        let mut rejected = Command::new("unused");
        assert_eq!(
            configure_project_config(&mut rejected, &workspace).unwrap_err(),
            "report_workspace_uncontrolled_configuration"
        );
        assert_eq!(rejected.get_args().count(), 0);
    }
}

#[test]
fn report_project_boundary_is_applied_by_the_official_harness() {
    use std::io::{BufRead as _, Write as _};
    let fixture = Fixture::new();
    let user_home = fixture.0.join("user-home");
    let workspace = user_home.join("Library/Application Support/profile/application-ai/policy");
    let harness_home = fixture.0.join("isolated-harness");
    for path in [
        user_home.join(".codex"),
        user_home.join(".git"),
        workspace.clone(),
        harness_home.clone(),
    ] {
        fs::create_dir_all(path).unwrap();
    }
    fs::write(user_home.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
    let user_config = user_home.join(".codex/config.toml");
    let contents = "model = \"uncontrolled-parent-model\"\n";
    fs::write(&user_config, contents).unwrap();
    let mut command = Command::new(crate::supervisor::codex_harness_test_binary());
    command
        .current_dir(&workspace)
        .env("CODEX_HOME", &harness_home)
        .env("CODEX_SQLITE_HOME", &harness_home)
        .env(crate::windows_sandbox::REMOTE_CONTROL_DISABLED_ENV, "1")
        .args([
            "--listen",
            "stdio://",
            "-c",
            "features.plugins=false",
            "-c",
            "check_for_update_on_startup=false",
        ]);
    configure_project_config(&mut command, &workspace).unwrap();
    let mut child = command
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let (sender, receiver) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        for line in std::io::BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            let Ok(value) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            let done = value["id"] == 2;
            if value.get("id").is_some() && sender.send(value).is_err() {
                break;
            }
            if done {
                break;
            }
        }
    });
    let response = (|| -> Result<Value, String> {
        let input = child.stdin.as_mut().unwrap();
        writeln!(input, "{}", json!({"id":1,"method":"initialize","params":{"clientInfo":{"name":"contexthub-desktop","version":"0.1.0"},"capabilities":{"experimentalApi":true}}})).map_err(|error| error.to_string())?;
        let initialized = receiver
            .recv_timeout(std::time::Duration::from_secs(10))
            .map_err(|error| error.to_string())?;
        if initialized["id"] != 1 || initialized.get("result").is_none() {
            return Err("report fixture initialization failed".into());
        }
        writeln!(input, "{}", json!({"method":"initialized","params":{}}))
            .map_err(|error| error.to_string())?;
        writeln!(
            input,
            "{}",
            json!({"id":2,"method":"config/read","params":{"cwd":workspace,"includeLayers":true}})
        )
        .map_err(|error| error.to_string())?;
        receiver
            .recv_timeout(std::time::Duration::from_secs(10))
            .map_err(|error| error.to_string())
    })();
    let _ = child.kill();
    child.wait().unwrap();
    reader.join().unwrap();
    let response = response.unwrap();
    assert_eq!(response["id"], 2);
    assert!(response.get("error").is_none(), "configuration read failed");
    assert_eq!(
        response["result"]["config"]["project_root_markers"],
        json!([])
    );
    assert_ne!(
        response["result"]["config"]["model"],
        "uncontrolled-parent-model"
    );
    let layers = response["result"]["layers"].as_array().unwrap();
    assert!(
        layers
            .iter()
            .all(|layer| layer["name"]["type"] != "project")
    );
    assert_eq!(fs::read_to_string(user_config).unwrap(), contents);
}

#[test]
fn controlled_materials_enforce_scope_external_file_grants_paging_and_operations() {
    let fixture = Fixture::new();
    let authorized = fixture.0.join("获准目录");
    fs::create_dir(&authorized).unwrap();
    let input = authorized.join("材料.txt");
    let outside = fixture.0.join("外部材料.txt");
    let neighbor = fixture.0.join("同级私密.txt");
    fs::write(&input, "资料\n".repeat(5000)).unwrap();
    fs::write(&outside, "用户明确选择的资料").unwrap();
    fs::write(&neighbor, "不可读").unwrap();
    let materials = Materials::new(std::slice::from_ref(&authorized)).unwrap();
    assert!(
        materials
            .call(&json!({"operation":"read","path":outside}))
            .is_err()
    );
    let first = materials
        .call(&json!({"operation":"read","path":input}))
        .unwrap();
    assert!(first["text"].as_str().unwrap().len() <= usize::try_from(MAX_READ_BYTES).unwrap());
    let next = materials
        .call(&json!({"operation":"read","path":input,"offset":first["nextOffset"]}))
        .unwrap();
    assert!(next["offset"].as_u64().unwrap() > 0);
    let mut roots = vec![authorized];
    roots.push(outside.canonicalize().unwrap());
    let materials = Materials::new(&roots).unwrap();
    assert_eq!(
        materials
            .call(&json!({"operation":"read","path":outside}))
            .unwrap()["text"],
        "用户明确选择的资料"
    );
    assert!(
        materials
            .call(&json!({"operation":"read","path":neighbor}))
            .is_err()
    );
    for operation in ["write", "exec", "delete", "shell"] {
        assert!(
            materials
                .call(&json!({"operation":operation,"path":input}))
                .is_err()
        );
    }
    assert!(
        materials
            .call(&json!({"operation":"read","path":input,"command":"cat"}))
            .is_err()
    );
    fs::write(&input, [0, 1, 2]).unwrap();
    assert_eq!(
        materials
            .call(&json!({"operation":"read","path":input}))
            .unwrap_err(),
        "report_workspace_text_required"
    );
}

#[test]
fn material_roots_and_open_handles_cannot_be_replaced_after_scope_check() {
    let fixture = Fixture::new();
    let root = fixture.0.join("root");
    fs::create_dir(&root).unwrap();
    let file = root.join("input.txt");
    fs::write(&file, "allowed").unwrap();
    let materials = Materials::new(std::slice::from_ref(&root)).unwrap();
    let (opened, identity) = materials.open(&file).unwrap();
    fs::rename(&file, root.join("original.txt")).unwrap();
    fs::write(&file, "replacement").unwrap();
    assert!(materials.verify_open(&file, &opened, &identity).is_err());
    fs::rename(&root, fixture.0.join("original-root")).unwrap();
    fs::create_dir(&root).unwrap();
    fs::write(&file, "outside-replacement").unwrap();
    assert!(
        materials
            .call(&json!({"operation":"read","path":file}))
            .is_err()
    );
}

#[cfg(unix)]
#[test]
fn material_symlinks_to_files_and_directories_outside_grants_are_denied() {
    let fixture = Fixture::new();
    let root = fixture.0.join("root");
    let outside = fixture.0.join("outside");
    fs::create_dir(&root).unwrap();
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("secret.txt"), "private").unwrap();
    std::os::unix::fs::symlink(&outside, root.join("escape")).unwrap();
    std::os::unix::fs::symlink(outside.join("secret.txt"), root.join("escape.txt")).unwrap();
    let materials = Materials::new(std::slice::from_ref(&root)).unwrap();
    for path in [root.join("escape/secret.txt"), root.join("escape.txt")] {
        assert_eq!(
            materials
                .call(&json!({"operation":"read","path":path}))
                .unwrap_err(),
            "report_workspace_path_denied"
        );
    }
    assert!(
        materials
            .call(&json!({"operation":"list","path":root.join("escape")}))
            .is_err()
    );
}

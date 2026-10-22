use crate::codex_harness::CodexHarnessSkill;
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::Path;

pub(crate) fn validate_configuration(response: &Value) -> Result<(), String> {
    let result = response.get("result").unwrap_or(response);
    let layers = result["layers"]
        .as_array()
        .ok_or("model_runtime_config_unconfirmed")?;
    for layer in layers {
        if layer["name"]["type"] == "system" {
            let values = layer["config"]
                .as_object()
                .ok_or("model_runtime_config_unconfirmed")?;
            if !values.is_empty() {
                return Err("model_runtime_system_config_not_isolated".into());
            }
        }
    }
    let config = &result["config"];
    if config["cli_auth_credentials_store"] != "file"
        || config["features"]["secret_auth_storage"] != false
        || config["features"]["system_proxy_fallback"] != false
    {
        return Err("model_runtime_auth_storage_not_isolated".into());
    }
    Ok(())
}

pub(crate) fn validate_tool_environment(response: &Value, home: &Path) -> Result<(), String> {
    let result = response.get("result").unwrap_or(response);
    let config = &result["config"];
    let environment = &config["shell_environment_policy"];
    let private_home = home.join("process-home");
    if config["features"]["shell_snapshot"] != false
        || config["features"]["shell_snapshot_v2"] != false
        || environment["inherit"] != "none"
        || environment["set"]["HOME"].as_str() != private_home.to_str()
        || environment["set"]["USERPROFILE"].as_str() != private_home.to_str()
        || environment["set"]["CODEX_HOME"].as_str() != home.to_str()
        || environment["set"].as_object().is_none_or(|values| {
            values.keys().any(|name| {
                name.contains("TOKEN") || name.contains("API_KEY") || name.contains("SECRET")
            })
        })
    {
        return Err("model_runtime_tool_environment_not_isolated".into());
    }
    Ok(())
}

pub(crate) fn validate_mcp(
    response: &Value,
    expected: &std::collections::BTreeMap<String, String>,
) -> Result<(), String> {
    let result = response.get("result").unwrap_or(response);
    let servers = &result["config"]["mcp_servers"];
    if servers.is_null() && expected.is_empty() {
        return Ok(());
    }
    let servers = servers.as_object().ok_or("model_runtime_mcp_unconfirmed")?;
    if servers.len() != expected.len()
        || expected.iter().any(|(name, url)| {
            servers.get(name).is_none_or(|server| {
                server["url"].as_str() != Some(url.as_str())
                    || server["enabled"] == false
                    || (name == "personal_user"
                        && (server["tool_timeout_sec"].as_f64() != Some(600.0)
                            || server["required"] != true
                            || server
                                .get("startup_readiness")
                                .is_some_and(|value| value != "connection")
                            || server
                                .get("supports_parallel_tool_calls")
                                .is_some_and(|value| value != false)))
            })
        })
    {
        return Err("model_runtime_mcp_not_authorized".into());
    }
    Ok(())
}

pub(crate) fn validate_skills(
    response: &Value,
    expected: &[CodexHarnessSkill],
) -> Result<(), String> {
    let expected = expected
        .iter()
        .map(|skill| canonical_skill(&skill.path))
        .collect::<Result<BTreeSet<_>, _>>()?;
    let result = response.get("result").unwrap_or(response);
    let entries = result["data"]
        .as_array()
        .ok_or("model_runtime_skills_unconfirmed")?;
    let mut enabled = BTreeSet::new();
    for entry in entries {
        if !entry["errors"].as_array().is_some_and(Vec::is_empty) {
            return Err("model_runtime_skills_unconfirmed".into());
        }
        for skill in entry["skills"]
            .as_array()
            .ok_or("model_runtime_skills_unconfirmed")?
        {
            match skill["enabled"].as_bool() {
                Some(false) => continue,
                Some(true) => {}
                None => return Err("model_runtime_skills_unconfirmed".into()),
            }
            let path = skill["path"]
                .as_str()
                .ok_or("model_runtime_skills_unconfirmed")?;
            let path = canonical_skill(Path::new(path))?;
            if !expected.contains(&path) {
                return Err("model_runtime_unapproved_skill".into());
            }
            enabled.insert(path);
        }
    }
    if enabled != expected {
        return Err("model_runtime_approved_skill_missing".into());
    }
    Ok(())
}

fn canonical_skill(path: &Path) -> Result<std::path::PathBuf, String> {
    if !path.is_absolute() || path.file_name().and_then(|value| value.to_str()) != Some("SKILL.md")
    {
        return Err("model_runtime_skill_path_invalid".into());
    }
    path.canonicalize()
        .map_err(|_| "model_runtime_skill_path_invalid".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn config() -> Value {
        json!({"layers":[{"name":{"type":"system"},"config":{}}],"config":{
            "cli_auth_credentials_store":"file","features":{"secret_auth_storage":false,"system_proxy_fallback":false}
        }})
    }

    #[test]
    fn system_configuration_is_rejected_without_exposing_its_values() {
        let mut value = config();
        value["layers"][0]["config"] = json!({"unknownPrivateValue":"must-not-be-printed"});
        assert_eq!(
            validate_configuration(&value).unwrap_err(),
            "model_runtime_system_config_not_isolated"
        );
        assert!(validate_configuration(&config()).is_ok());
    }

    #[test]
    fn system_or_automatic_credential_storage_is_rejected() {
        for (pointer, replacement) in [
            ("/config/cli_auth_credentials_store", json!("auto")),
            ("/config/cli_auth_credentials_store", json!("keyring")),
            ("/config/cli_auth_credentials_store", json!("ephemeral")),
            ("/config/features/secret_auth_storage", json!(true)),
            ("/config/features/system_proxy_fallback", json!(true)),
        ] {
            let mut value = config();
            *value.pointer_mut(pointer).unwrap() = replacement;
            assert_eq!(
                validate_configuration(&value).unwrap_err(),
                "model_runtime_auth_storage_not_isolated"
            );
        }
    }

    #[test]
    fn locked_mcp_serialization_accepts_omitted_safe_defaults_and_rejects_other_values() {
        let expected = std::collections::BTreeMap::from([(
            "personal_user".into(),
            "http://127.0.0.1:1/mcp".into(),
        )]);
        let base = json!({"config":{"mcp_servers":{"personal_user":{
            "url":"http://127.0.0.1:1/mcp","enabled":true,"required":true,"tool_timeout_sec":600.0
        }}}});
        assert!(validate_mcp(&base, &expected).is_ok());
        let mut explicit = base.clone();
        explicit["config"]["mcp_servers"]["personal_user"]["startup_readiness"] =
            json!("connection");
        explicit["config"]["mcp_servers"]["personal_user"]["supports_parallel_tool_calls"] =
            json!(false);
        assert!(validate_mcp(&explicit, &expected).is_ok());
        for (field, value) in [
            ("startup_readiness", json!("catalog")),
            ("startup_readiness", Value::Null),
            ("supports_parallel_tool_calls", json!(true)),
            ("supports_parallel_tool_calls", Value::Null),
            ("required", json!(false)),
            ("tool_timeout_sec", json!(60.0)),
        ] {
            let mut config = base.clone();
            config["config"]["mcp_servers"]["personal_user"][field] = value;
            assert_eq!(
                validate_mcp(&config, &expected).unwrap_err(),
                "model_runtime_mcp_not_authorized"
            );
        }
    }

    #[test]
    fn unknown_enabled_skill_is_never_treated_as_approved() {
        let root = std::env::temp_dir().join(format!(
            "cilai-personal-skill-policy-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("SKILL.md");
        std::fs::write(
            &path,
            "---\nname: policy-fixture\ndescription: fixture\n---\n",
        )
        .unwrap();
        let list =
            |enabled| json!({"data":[{"skills":[{"path":path,"enabled":enabled}],"errors":[]}]});
        assert_eq!(
            validate_skills(&list(true), &[]).unwrap_err(),
            "model_runtime_unapproved_skill"
        );
        assert!(validate_skills(&list(false), &[]).is_ok());
        let approved = CodexHarnessSkill {
            skill_id: "fixture".into(),
            name: "fixture".into(),
            path: path.clone(),
            explicit: false,
        };
        assert!(validate_skills(&list(true), std::slice::from_ref(&approved)).is_ok());
        assert_eq!(
            validate_skills(&list(false), &[approved]).unwrap_err(),
            "model_runtime_approved_skill_missing"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}

use contexthub_protocol::{ModelAuthMode, ModelConnectionKind, ModelExecutionSnapshot};

pub(super) fn validate(
    foreground: &ModelExecutionSnapshot,
    auxiliary: &ModelExecutionSnapshot,
) -> Result<(), String> {
    if foreground.profile_id != auxiliary.profile_id
        || foreground.instance_fence != auxiliary.instance_fence
        || foreground.runtime_kind != auxiliary.runtime_kind
    {
        return Err("model_auxiliary_runtime_scope_mismatch".into());
    }
    let native_auth = |snapshot: &ModelExecutionSnapshot| {
        snapshot.kind == ModelConnectionKind::Chatgpt
            || snapshot.auth_mode == ModelAuthMode::RuntimeManagedChatgpt
    };
    if (native_auth(foreground) || native_auth(auxiliary))
        && (foreground.connection_id != auxiliary.connection_id
            || foreground.config_revision != auxiliary.config_revision
            || foreground.auth_generation != auxiliary.auth_generation
            || foreground.runtime_home_scope != auxiliary.runtime_home_scope
            || foreground.kind != auxiliary.kind
            || foreground.auth_mode != auxiliary.auth_mode
            || foreground.network != auxiliary.network)
    {
        return Err("model_auxiliary_runtime_managed_auth_not_supported".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use contexthub_protocol::{ModelNetworkConfig, ModelPurpose, ModelRuntimeKind};

    fn snapshot() -> ModelExecutionSnapshot {
        ModelExecutionSnapshot {
            validation_id: None,
            runtime_kind: ModelRuntimeKind::Workbench,
            profile_id: "profile".into(),
            instance_fence: "instance".into(),
            connection_id: "foreground".into(),
            kind: ModelConnectionKind::OpenaiKey,
            config_revision: 1,
            auth_generation: 1,
            purpose: ModelPurpose::Foreground,
            model_id: "foreground-model".into(),
            reasoning_effort: None,
            base_url: None,
            local_runtime: None,
            auth_mode: ModelAuthMode::ApiKey,
            network: ModelNetworkConfig::default(),
            secret_ref: None,
            runtime_home_scope: "foreground-home".into(),
            descriptor_revision: 1,
            capabilities: std::collections::BTreeMap::new(),
            context_window_tokens: None,
            input_modalities: vec!["text".into()],
            default_reasoning_effort: None,
            supported_reasoning_efforts: Vec::new(),
            binding_revision: None,
            binding_id: None,
            conversation_id: None,
        }
    }

    #[test]
    fn responses_auxiliary_connections_keep_independent_identity_and_scope() {
        let foreground = snapshot();
        let mut auxiliary = foreground.clone();
        auxiliary.connection_id = "review".into();
        auxiliary.purpose = ModelPurpose::Review;
        auxiliary.config_revision = 7;
        auxiliary.auth_generation = 3;
        auxiliary.runtime_home_scope = "review-home".into();
        auxiliary.model_id = "review-model".into();
        auxiliary.base_url = Some("http://127.0.0.1:1234/v1".into());
        assert_eq!(validate(&foreground, &auxiliary), Ok(()));
        auxiliary.profile_id = "another-profile".into();
        assert_eq!(
            validate(&foreground, &auxiliary).unwrap_err(),
            "model_auxiliary_runtime_scope_mismatch"
        );
    }

    #[test]
    fn native_chatgpt_auth_requires_the_same_provider_home_and_auth_generation() {
        let mut foreground = snapshot();
        foreground.kind = ModelConnectionKind::Chatgpt;
        foreground.auth_mode = ModelAuthMode::RuntimeManagedChatgpt;
        let mut auxiliary = foreground.clone();
        auxiliary.purpose = ModelPurpose::Review;
        auxiliary.model_id = "review-model".into();
        assert_eq!(validate(&foreground, &auxiliary), Ok(()));
        for field in ["connection", "home", "auth", "provider"] {
            let mut changed = auxiliary.clone();
            match field {
                "connection" => changed.connection_id = "other".into(),
                "home" => changed.runtime_home_scope = "other-home".into(),
                "auth" => changed.auth_generation += 1,
                "provider" => changed.kind = ModelConnectionKind::OpenaiKey,
                _ => unreachable!(),
            }
            assert_eq!(
                validate(&foreground, &changed).unwrap_err(),
                "model_auxiliary_runtime_managed_auth_not_supported"
            );
        }
        let api = snapshot();
        assert!(validate(&api, &auxiliary).is_err());
        assert!(validate(&foreground, &api).is_err());
    }
}

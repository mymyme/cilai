use contexthub_protocol::PersonalPageBindingParams;
use contexthub_workbench_runtime::personal_page_ai::{PersonalPageAiManager, UiActionExecutor};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::{Arc, OnceLock};

static MANAGER: OnceLock<PersonalPageAiManager> = OnceLock::new();

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Register {
    document_id: String,
    binding: PersonalPageBindingParams,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Dispatch {
    document_id: String,
    request: Invocation,
}

#[derive(Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Invocation {
    method: String,
    request_id: String,
    params: Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Resolve {
    document_id: String,
    run_id: String,
    confirmation_id: String,
    decision: String,
    arguments: Option<Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Document {
    document_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}

fn decode<T: serde::de::DeserializeOwned>(value: &Value) -> Result<T, String> {
    if serde_json::to_vec(value)
        .map_err(|_| "personal_page_request_invalid")?
        .len()
        > 256 * 1024
    {
        return Err("personal_page_request_too_large".into());
    }
    serde_json::from_value(value.clone()).map_err(|_| "personal_page_request_invalid".into())
}

pub fn is_command(command: &str) -> bool {
    matches!(
        command,
        "personal_page_ai_register_internal"
            | "personal_page_ai_dispatch_internal"
            | "personal_page_ai_resolve_internal"
            | "personal_page_ai_close_internal"
            | "personal_page_ai_revoke_all_internal"
    )
}

fn manager() -> &'static PersonalPageAiManager {
    MANAGER.get_or_init(|| PersonalPageAiManager::new(crate::ai_manager().clone()))
}

fn ui_action(document: String) -> UiActionExecutor {
    Arc::new(move |action, arguments| {
        crate::host_bridge().request(
            "personal_page_ai_ui_action",
            &json!({"documentId":document,"actionId":action,"arguments":arguments}),
        )
    })
}

pub fn revoke_all() -> Result<Value, String> {
    confirmed_exit(manager().revoke_all()?)
}

fn confirmed_exit(result: Value) -> Result<Value, String> {
    if result["retired"] != true || result["processesExited"] != true {
        return Err("personal_page_process_exit_unconfirmed".into());
    }
    Ok(result)
}

pub fn handle(command: &str, args: &Value) -> Result<Value, String> {
    match command {
        "personal_page_ai_register_internal" => {
            let p: Register = decode(args)?;
            manager().register_document(&p.document_id, p.binding)
        }
        "personal_page_ai_dispatch_internal" => {
            let p: Dispatch = decode(args)?;
            if p.request.method == "model.write.resolve" {
                return Err("personal_page_confirmation_host_required".into());
            }
            if !p.request.params.is_object() {
                return Err("personal_page_request_invalid".into());
            }
            let request =
                serde_json::to_value(p.request).map_err(|_| "personal_page_request_invalid")?;
            let ui = ui_action(p.document_id.clone());
            manager().dispatch(&p.document_id, request, ui)
        }
        "personal_page_ai_resolve_internal" => {
            let p: Resolve = decode(args)?;
            if !matches!(p.decision.as_str(), "approved" | "rejected")
                || p.arguments.as_ref().is_some_and(|args| !args.is_object())
            {
                return Err("personal_page_request_invalid".into());
            }
            manager().resolve_write(
                &p.document_id,
                &p.run_id,
                &p.confirmation_id,
                &p.decision,
                p.arguments.as_ref(),
            )
        }
        "personal_page_ai_close_internal" => {
            let p: Document = decode(args)?;
            manager().close_document(&p.document_id)
        }
        "personal_page_ai_revoke_all_internal" => {
            let _: Empty = decode(args)?;
            revoke_all()
        }
        _ => Err("personal_page_command_not_allowed".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::{Dispatch, Empty, Register, Resolve, confirmed_exit, decode, handle, is_command};
    use serde_json::json;

    #[test]
    fn private_commands_do_not_accept_caller_paths_permissions_or_owner() {
        for key in [
            "owner",
            "profileId",
            "packageRoot",
            "dataRoot",
            "permissionDigest",
            "sessionId",
        ] {
            let mut register = json!({"documentId":"doc","binding":{"creationProjectId":"one","expectedPackageDigest":"digest"}});
            register[key] = json!("injected");
            assert!(decode::<Register>(&register).is_err());
            let mut binding = register;
            binding.as_object_mut().unwrap().remove(key);
            binding["binding"][key] = json!("injected");
            assert!(decode::<Register>(&binding).is_err());
        }
        assert!(decode::<Empty>(&json!({"documentId":"doc"})).is_err());
    }

    #[test]
    fn a_page_dispatch_cannot_resolve_its_own_confirmation() {
        assert_eq!(handle("personal_page_ai_dispatch_internal", &json!({"documentId":"doc","request":{"method":"model.write.resolve","requestId":"request","params":{"decision":"approve"}}})).unwrap_err(),"personal_page_confirmation_host_required");
        assert!(decode::<Dispatch>(&json!({"documentId":"doc","request":{"method":"session.read","requestId":"request","params":{},"binding":{}}})).is_err());
        assert!(decode::<Resolve>(&json!({"documentId":"doc","runId":"run","confirmationId":"confirm","decision":"approve","owner":{}})).is_err());
    }

    #[test]
    fn unknown_commands_and_oversized_requests_are_refused_before_runtime() {
        assert!(!is_command("personal_page_ai_dispatch"));
        assert_eq!(
            handle("personal_page_ai_dispatch", &json!({})).unwrap_err(),
            "personal_page_command_not_allowed"
        );
        assert!(decode::<Dispatch>(&json!({"documentId":"doc","request":{"method":"context.publish","requestId":"request","params":{"text":"x".repeat(256*1024)}}})).is_err());
        assert_eq!(handle("personal_page_ai_resolve_internal",&json!({"documentId":"doc","runId":"run","confirmationId":"confirmation","decision":"automatic"})).unwrap_err(),"personal_page_request_invalid");
    }

    #[test]
    fn cleanup_pending_is_preserved_but_unconfirmed_process_exit_cannot_pass_restart() {
        let receipt = json!({"retired":true,"processesExited":true,"cleanupPending":true,"auditPending":true,"documents":[]});
        assert_eq!(confirmed_exit(receipt.clone()).unwrap(), receipt);
        for invalid in [
            json!({}),
            json!({"retired":true,"processesExited":false}),
            json!({"retired":false,"processesExited":true}),
        ] {
            assert_eq!(
                confirmed_exit(invalid).unwrap_err(),
                "personal_page_process_exit_unconfirmed"
            );
        }
    }
}

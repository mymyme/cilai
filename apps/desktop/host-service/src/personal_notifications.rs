use contexthub_workbench_runtime::ai_runtime::personal_notifications::{
    NotificationClaimParams, NotificationPreferencesSave, NotificationReportParams,
};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request<T> {
    request: T,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Open {
    profile_id: String,
    notification_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}

fn decode<T: serde::de::DeserializeOwned>(value: &Value) -> Result<T, String> {
    if serde_json::to_vec(value)
        .map_err(|_| "personal_notification_request_invalid")?
        .len()
        > 16 * 1024
    {
        return Err("personal_notification_request_too_large".into());
    }
    serde_json::from_value(value.clone())
        .map_err(|_| "personal_notification_request_invalid".into())
}

pub fn is_command(command: &str) -> bool {
    matches!(
        command,
        "personal_notifications_read_internal"
            | "personal_notifications_claim_internal"
            | "personal_notifications_report_internal"
            | "personal_notifications_open_internal"
            | "personal_notification_preferences_get"
            | "personal_notification_preferences_save"
    )
}

pub fn handle(command: &str, args: &Value) -> Result<Value, String> {
    let manager = crate::ai_manager();
    match command {
        "personal_notifications_read_internal" => {
            let _: Empty = decode(args)?;
            crate::encode(manager.personal_notification_snapshot()?)
        }
        "personal_notifications_claim_internal" => {
            let params: Request<NotificationClaimParams> = decode(args)?;
            crate::encode(manager.claim_personal_notification(params.request)?)
        }
        "personal_notifications_report_internal" => {
            let params: Request<NotificationReportParams> = decode(args)?;
            crate::encode(manager.report_personal_notification(params.request)?)
        }
        "personal_notifications_open_internal" => {
            let params: Request<Open> = decode(args)?;
            crate::encode(manager.open_personal_notification(
                &params.request.profile_id,
                &params.request.notification_id,
            )?)
        }
        "personal_notification_preferences_get" => {
            let _: Empty = decode(args)?;
            let state = manager.personal_notification_snapshot()?;
            Ok(json!({"profileId":state.profile_id,"preferences":state.preferences}))
        }
        "personal_notification_preferences_save" => {
            let params: Request<NotificationPreferencesSave> = decode(args)?;
            let profile = params.request.profile_id.clone();
            let preferences = manager.save_personal_notification_preferences(&params.request)?;
            Ok(json!({"profileId":profile,"preferences":preferences}))
        }
        _ => Err("personal_notification_command_invalid".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notification_commands_reject_untrusted_text_and_unbounded_arguments() {
        assert!(decode::<Empty>(&json!({"title":"not accepted"})).is_err());
        assert!(decode::<Request<NotificationClaimParams>>(&json!({"request":{
            "profileId":"a".repeat(32),"sourceEpoch":"epoch","eventId":"b".repeat(64),"body":"not accepted"
        }})).is_err());
        assert!(decode::<Request<NotificationReportParams>>(&json!({"request":{
            "profileId":"a".repeat(32),"notificationId":"b".repeat(32),"outcome":"unknown","reasonCode":null,
            "arguments":"not accepted"
        }})).is_err());
        assert!(decode::<Value>(&json!({"large":"x".repeat(17000)})).is_err());
    }
}

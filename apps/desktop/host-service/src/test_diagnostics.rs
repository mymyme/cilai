//! Bounded read-only export of known sandbox error types. Never serialize log
//! lines, credentials, conversations, database data, or project file contents.
use serde_json::{Value, json};
use std::io::{Read as _, Seek as _, SeekFrom};
use std::path::Path;

fn classify(line: &str) -> Vec<&'static str> {
    [
        (
            "sandbox_refresh_failed",
            line.contains("setup refresh had errors")
                || line.contains("setup refresh completed with errors:"),
        ),
        (
            "workspace_acl_access_denied",
            line.contains("write ACE") && line.contains("SetNamedSecurityInfoW failed: 5"),
        ),
        (
            "runtime_acl_failed",
            line.contains("runtime read/execute") && line.contains("failed on"),
        ),
        (
            "sandbox_deny_acl_failed",
            line.contains("deny ACE failed on"),
        ),
        (
            "sandbox_helper_unknown_error",
            line.contains("helper_unknown_error"),
        ),
        (
            "runtime_directory_owner_mismatch",
            line.contains("runtime directory is not owned"),
        ),
    ]
    .into_iter()
    .filter_map(|(code, found)| found.then_some(code))
    .collect()
}

fn errors_in_file(path: &Path) -> Value {
    let result = (|| -> std::io::Result<_> {
        let metadata = std::fs::symlink_metadata(path)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(std::io::Error::other("invalid log"));
        }
        let mut file = std::fs::File::open(path)?;
        let length = file.metadata()?.len();
        file.seek(SeekFrom::Start(length.saturating_sub(128 * 1024)))?;
        let mut bytes = Vec::new();
        file.take(128 * 1024).read_to_end(&mut bytes)?;
        let text = String::from_utf8_lossy(&bytes);
        let mut errors = text
            .lines()
            .filter_map(|line| {
                let codes = classify(line);
                if codes.is_empty() {
                    return None;
                }
                let time = line
                    .strip_prefix('[')
                    .and_then(|value| value.split(']').next())
                    .filter(|value| {
                        value.len() <= 40
                            && value.chars().all(|character| {
                                character.is_ascii_digit() || "-T:+.Z ".contains(character)
                            })
                    });
                Some(json!({"at":time,"codes":codes}))
            })
            .rev()
            .take(24)
            .collect::<Vec<_>>();
        errors.reverse();
        Ok(errors)
    })();
    match result {
        Ok(errors) => {
            json!({"source":path.file_name().and_then(|name| name.to_str()),"status":"read","errors":errors})
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => json!({"status":"absent"}),
        Err(error) => json!({"status":"unreadable","osError":error.raw_os_error()}),
    }
}

pub(super) fn collect() -> Value {
    let profile = super::current_profile().ok();
    collect_for_profile(
        profile.as_ref().map(|profile| profile.data_dir.as_path()),
        &super::startup_permissions::diagnostics(),
    )
}

fn collect_for_profile(profile: Option<&Path>, startup: &Value) -> Value {
    let Some(profile) = profile else {
        // Startup may fail before active_profile is assigned. Report that
        // state explicitly; do not guess a tenant or scan other profiles.
        return json!({"status":"collected","profileStatus":"unavailable","startup":startup,"sandbox":[]});
    };
    let record = profile.join("run/instance.json");
    let instance = match std::fs::symlink_metadata(&record) {
        Ok(_) => {
            match contexthub_platform_adapter::security::verify_current_user_private_file(&record) {
                Ok(()) => json!({"privateFileValidation":"passed"}),
                Err(error) => {
                    json!({"privateFileValidation":"failed","ownerMismatch":error.to_string().contains("not owned by the current user"),"osError":error.raw_os_error()})
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            json!({"privateFileValidation":"absent"})
        }
        Err(error) => json!({"privateFileValidation":"unreadable","osError":error.raw_os_error()}),
    };
    let home = profile.join("ai-runtime/cilai-harness");
    let mut logs = vec![
        errors_in_file(&home.join("windows-sandbox-readiness.stderr.log")),
        errors_in_file(&home.join("windows-sandbox-setup.stderr.log")),
    ];
    // Date-derived filenames only, no recursive scanning or untrusted path input.
    for days in 0..4 {
        let date = chrono::Utc::now().date_naive() - chrono::Duration::days(days);
        logs.push(errors_in_file(
            &home.join(format!(".sandbox/sandbox.{date}.log")),
        ));
    }
    json!({"status":"collected","profileStatus":"active","startup":startup,"instance":instance,"sandbox":logs})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn extracts_error_types_without_exposing_paths_or_arbitrary_lines() {
        assert_eq!(
            classify("write ACE grant failed on F:\\私密目录: SetNamedSecurityInfoW failed: 5"),
            vec!["workspace_acl_access_denied"]
        );
        assert!(classify("token=secret conversation=private").is_empty());
        assert_eq!(
            classify("setup refresh had errors"),
            vec!["sandbox_refresh_failed"]
        );
    }

    #[test]
    fn initialization_failure_does_not_discard_startup_diagnostics_or_guess_a_profile() {
        let startup = json!({"status":"collected","context":{"processElevated":true,"errors":[{"scope":"profile","code":"private_file_owner_mismatch","ownerCategory":"administrators"}]}});
        let result = collect_for_profile(None, &startup);
        assert_eq!(result["status"], "collected");
        assert_eq!(result["profileStatus"], "unavailable");
        assert_eq!(result["startup"], startup);
        assert_eq!(result["sandbox"], json!([]));
        assert!(result.get("instance").is_none());
    }
}

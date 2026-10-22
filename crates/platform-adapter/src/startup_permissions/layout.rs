use std::path::{Path, PathBuf};

const APPLICATION_ID: &str = "cn.cilai.personal.workbench";
const BUSINESS_DIRECTORY: &str = "business";
const RECOVERY_DIRECTORY: &str = "cn.cilai.personal.workbench-recovery";

pub(super) fn valid_scope(environment: &str, profile_id: &str) -> bool {
    environment == BUSINESS_DIRECTORY
        && profile_id.len() == 32
        && profile_id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(super) fn root(local_data: &Path) -> PathBuf {
    local_data.join(APPLICATION_ID).join(BUSINESS_DIRECTORY)
}

pub(super) fn recovery(local_data: &Path) -> PathBuf {
    local_data.join(RECOVERY_DIRECTORY)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_paths_are_confined_to_personal_application_and_recovery() {
        let local = Path::new("LocalAppData");
        assert_eq!(
            root(local),
            local.join("cn.cilai.personal.workbench/business")
        );
        assert_eq!(
            recovery(local),
            local.join("cn.cilai.personal.workbench-recovery")
        );
        assert!(!root(local).starts_with(local.join("com.cilai.workbench")));
    }

    #[test]
    fn startup_scope_rejects_enterprise_profiles_and_environment_names() {
        let id = "0123456789abcdef0123456789abcdef";
        assert!(valid_scope("business", id));
        for environment in ["dev", "prod", "", "../business"] {
            assert!(!valid_scope(environment, id));
        }
        for profile in [
            "legacy",
            "signed-out",
            "../../Windows",
            "0123456789ABCDEF0123456789ABCDEF",
            "unresolved-personal-profile",
            "",
        ] {
            assert!(!valid_scope("business", profile));
        }
    }
}

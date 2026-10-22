//! Explicit desktop changes to machine-wide script policy. No arbitrary shell,
//! path, command or policy text crosses the elevation boundary.
use serde::{Deserialize, Serialize};
/// Whether the host already has an elevated token. Does not request elevation.
/// # Errors
/// Returns a native token-query error if Windows cannot report the token state.
pub fn process_elevated() -> std::io::Result<bool> {
    #[cfg(windows)]
    {
        let token = crate::workspace_permissions::windows::current_token()?;
        crate::workspace_permissions::windows::elevated(&token)
    }
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    Ok(false)
}
use std::io;

pub const HELPER_ARGUMENT: &str = "--change-powershell-policy";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum Policy {
    Undefined,
    Restricted,
    AllSigned,
    RemoteSigned,
    Unrestricted,
    Bypass,
}
impl Policy {
    #[must_use]
    pub const fn permits_local_scripts(self) -> bool {
        matches!(self, Self::RemoteSigned | Self::Unrestricted | Self::Bypass)
    }
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Engine {
    WindowsPowerShell,
    PowerShell7,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Scopes {
    pub machine_policy: Policy,
    pub user_policy: Policy,
    pub current_user: Policy,
    pub local_machine: Policy,
    pub effective: Policy,
}
impl Scopes {
    #[must_use]
    pub fn managed(&self) -> bool {
        self.machine_policy != Policy::Undefined || self.user_policy != Policy::Undefined
    }
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EngineState {
    pub engine: Engine,
    pub path: String,
    pub identity: String,
    pub scopes: Scopes,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Change {
    engine: Engine,
    identity: String,
    before: Policy,
    after: Policy,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ChangePlan {
    changes: Vec<Change>,
    parent_pid: u32,
    parent_fingerprint: String,
    user_sid: String,
    expires_at: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PolicyChange {
    pub engine: Engine,
    pub before: Policy,
    pub after: Policy,
}
impl ChangePlan {
    #[must_use]
    pub fn preview(&self) -> Vec<PolicyChange> {
        self.changes
            .iter()
            .map(|change| PolicyChange {
                engine: change.engine,
                before: change.before,
                after: change.after,
            })
            .collect()
    }
}

/// Read standard system installations without modifying policy or requesting UAC.
/// # Errors
/// Unsupported platforms, unsafe installations and failed queries are explicit.
pub fn inspect() -> io::Result<Vec<EngineState>> {
    #[cfg(windows)]
    return windows::inspect();
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "powershell_policy_unsupported",
    ))
}

/// Bind a short-lived change to this normal desktop process. Restore values are
/// only the original restrictive policies changed by this feature.
/// # Errors
/// Managed policy, replaced installations or conflicting state reject the plan.
pub fn prepare(
    states: &[EngineState],
    restore: Option<&[EngineState]>,
) -> io::Result<Option<ChangePlan>> {
    let changes = changes(states, restore)?;
    if changes.is_empty() {
        return Ok(None);
    }
    #[cfg(windows)]
    return windows::plan(changes).map(Some);
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "powershell_policy_unsupported",
    ))
}

fn changes(states: &[EngineState], restore: Option<&[EngineState]>) -> io::Result<Vec<Change>> {
    let mut output = Vec::new();
    for state in states {
        let before = state.scopes.local_machine;
        let after = if let Some(original) = restore {
            let old = original
                .iter()
                .find(|old| old.engine == state.engine)
                .ok_or_else(|| io::Error::other("powershell_policy_conflict"))?;
            if old.identity != state.identity {
                return Err(io::Error::other("powershell_policy_conflict"));
            }
            let target = old.scopes.local_machine;
            if before == target || target.permits_local_scripts() {
                continue;
            }
            if before != Policy::RemoteSigned {
                return Err(io::Error::other("powershell_policy_conflict"));
            }
            target
        } else {
            if before.permits_local_scripts() || state.scopes.managed() {
                continue;
            }
            Policy::RemoteSigned
        };
        if state.scopes.managed() {
            return Err(io::Error::other("powershell_policy_managed"));
        }
        output.push(Change {
            engine: state.engine,
            identity: state.identity.clone(),
            before,
            after,
        });
    }
    if restore.is_some_and(|old| {
        old.iter()
            .any(|old| !states.iter().any(|s| s.engine == old.engine))
    }) {
        return Err(io::Error::other("powershell_policy_conflict"));
    }
    Ok(output)
}

/// Elevate the dedicated helper, then re-read machine settings as the original user.
/// # Errors
/// Cancellation, expired plans, rollback failure and failed verification are explicit.
pub fn apply(plan: &ChangePlan) -> io::Result<Vec<EngineState>> {
    #[cfg(windows)]
    return windows::apply(plan);
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        let _ = plan;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "powershell_policy_unsupported",
        ))
    }
}

#[must_use]
pub fn helper_exit_code(arguments: &[String]) -> Option<i32> {
    if arguments.first().map(String::as_str) != Some(HELPER_ARGUMENT) {
        return None;
    }
    #[cfg(windows)]
    return Some(windows::helper(arguments).map_or_else(|e| e.raw_os_error().unwrap_or(1), |()| 0));
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    Some(1)
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod windows;

#[cfg(any(windows, test))]
fn transaction(
    changes: &[Change],
    mut read: impl FnMut(usize) -> io::Result<Policy>,
    mut write: impl FnMut(usize, Policy) -> io::Result<()>,
) -> io::Result<()> {
    let mut touched = 0;
    let result = (|| {
        for (index, change) in changes.iter().enumerate() {
            if read(index)? != change.before {
                return Err(io::Error::from_raw_os_error(1237));
            }
            touched = index + 1;
            write(index, change.after)?;
            if read(index)? != change.after {
                return Err(io::Error::other("powershell_policy_verification_failed"));
            }
        }
        Ok(())
    })();
    if let Err(error) = result {
        for index in (0..touched).rev() {
            let change = &changes[index];
            let current = read(index).map_err(|_| io::Error::from_raw_os_error(10005))?;
            if current == change.before {
                continue;
            }
            if current != change.after
                || write(index, change.before).is_err()
                || !read(index).is_ok_and(|p| p == change.before)
            {
                return Err(io::Error::from_raw_os_error(10005));
            }
        }
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn state(policy: Policy) -> EngineState {
        EngineState {
            engine: Engine::WindowsPowerShell,
            path: "fixed".into(),
            identity: "same".into(),
            scopes: Scopes {
                machine_policy: Policy::Undefined,
                user_policy: Policy::Undefined,
                current_user: Policy::Undefined,
                local_machine: policy,
                effective: policy,
            },
        }
    }
    #[test]
    fn enables_local_scripts_without_downgrading_existing_or_managed_policies() {
        for old in [Policy::Undefined, Policy::Restricted, Policy::AllSigned] {
            let result = changes(&[state(old)], None).unwrap();
            assert_eq!(result.len(), 1);
            assert_eq!(result[0].after, Policy::RemoteSigned);
        }
        for old in [Policy::RemoteSigned, Policy::Unrestricted, Policy::Bypass] {
            assert!(changes(&[state(old)], None).unwrap().is_empty());
        }
        let mut managed = state(Policy::Restricted);
        managed.scopes.machine_policy = Policy::AllSigned;
        assert!(changes(&[managed], None).unwrap().is_empty());
    }
    #[test]
    fn restores_only_our_remote_signed_change_and_rejects_external_changes() {
        let old = [state(Policy::Undefined)];
        assert_eq!(
            changes(&[state(Policy::RemoteSigned)], Some(&old)).unwrap()[0].after,
            Policy::Undefined
        );
        assert!(
            changes(&[state(Policy::Undefined)], Some(&old))
                .unwrap()
                .is_empty()
        );
        assert!(changes(&[state(Policy::Bypass)], Some(&old)).is_err());
        let mut changed = state(Policy::RemoteSigned);
        changed.identity = "different".into();
        assert!(changes(&[changed], Some(&old)).is_err());
        let mut managed = state(Policy::RemoteSigned);
        managed.scopes.user_policy = Policy::Restricted;
        assert!(changes(&[managed], Some(&old)).is_err());
    }
    #[test]
    fn multi_engine_failure_rolls_back_even_a_write_that_failed_after_changing_state() {
        use std::cell::RefCell;
        let states = RefCell::new(vec![Policy::Restricted, Policy::AllSigned]);
        let mut list = changes(&[state(Policy::Restricted)], None).unwrap();
        let mut second = list[0].clone();
        second.engine = Engine::PowerShell7;
        second.before = Policy::AllSigned;
        list.push(second);
        let result = transaction(
            &list,
            |index| Ok(states.borrow()[index]),
            |index, value| {
                states.borrow_mut()[index] = value;
                if index == 1 && value == Policy::RemoteSigned {
                    return Err(io::Error::other("write failed"));
                }
                Ok(())
            },
        );
        assert!(result.is_err());
        assert_eq!(
            *states.borrow(),
            vec![Policy::Restricted, Policy::AllSigned]
        );
    }
    #[test]
    fn rollback_failure_and_external_conflict_are_not_reported_as_success() {
        use std::cell::Cell;
        let value = Cell::new(Policy::Restricted);
        let list = changes(&[state(Policy::Restricted)], None).unwrap();
        let error = transaction(
            &list,
            |_| Ok(value.get()),
            |_, p| {
                if p == Policy::Restricted {
                    return Err(io::Error::other("rollback denied"));
                }
                value.set(p);
                Err(io::Error::other("failed after write"))
            },
        )
        .unwrap_err();
        assert_eq!(error.raw_os_error(), Some(10005));
        let calls = Cell::new(0);
        assert!(
            transaction(
                &list,
                |_| Ok(Policy::Bypass),
                |_, _| {
                    calls.set(calls.get() + 1);
                    Ok(())
                }
            )
            .is_err()
        );
        assert_eq!(calls.get(), 0);
    }
}

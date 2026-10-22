#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FaultAction {
    Abort,
}

#[must_use]
pub fn configured_action(point: &str) -> Option<FaultAction> {
    let enabled = std::env::var("CONTEXTHUB_ENABLE_FAULT_INJECTION")
        .ok()
        .as_deref()
        == Some("1");
    let configured = std::env::var("CONTEXTHUB_FAULT_POINT").ok();
    (enabled && configured.as_deref() == Some(point)).then_some(FaultAction::Abort)
}

pub fn terminate_if_configured(point: &str) {
    if configured_action(point) == Some(FaultAction::Abort) {
        std::process::abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_by_default() {
        assert_eq!(configured_action("before_transaction"), None);
    }
}

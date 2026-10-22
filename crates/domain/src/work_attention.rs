#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttentionCaseType {
    ProjectBlocker,
    DecisionRequest,
    AgentRunIncident,
}

impl AttentionCaseType {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ProjectBlocker => "project_blocker",
            Self::DecisionRequest => "decision_request",
            Self::AgentRunIncident => "agent_run_incident",
        }
    }
}

impl TryFrom<&str> for AttentionCaseType {
    type Error = ();

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "project_blocker" => Ok(Self::ProjectBlocker),
            "decision_request" => Ok(Self::DecisionRequest),
            "agent_run_incident" => Ok(Self::AgentRunIncident),
            _ => Err(()),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttentionCaseStatus {
    Open,
    Acknowledged,
    Resolved,
}

impl AttentionCaseStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Acknowledged => "acknowledged",
            Self::Resolved => "resolved",
        }
    }

    #[must_use]
    pub const fn can_transition_to(self, case_type: AttentionCaseType, target: Self) -> bool {
        matches!(
            (case_type, self, target),
            (
                AttentionCaseType::ProjectBlocker | AttentionCaseType::DecisionRequest,
                Self::Open,
                Self::Resolved
            ) | (
                AttentionCaseType::AgentRunIncident,
                Self::Open,
                Self::Acknowledged | Self::Resolved
            ) | (
                AttentionCaseType::AgentRunIncident,
                Self::Acknowledged,
                Self::Resolved
            )
        )
    }
}

impl TryFrom<&str> for AttentionCaseStatus {
    type Error = ();

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "open" => Ok(Self::Open),
            "acknowledged" => Ok(Self::Acknowledged),
            "resolved" => Ok(Self::Resolved),
            _ => Err(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attention_cases_have_type_specific_transitions() {
        assert!(AttentionCaseStatus::Open.can_transition_to(
            AttentionCaseType::ProjectBlocker,
            AttentionCaseStatus::Resolved,
        ));
        assert!(!AttentionCaseStatus::Open.can_transition_to(
            AttentionCaseType::ProjectBlocker,
            AttentionCaseStatus::Acknowledged,
        ));
        assert!(AttentionCaseStatus::Open.can_transition_to(
            AttentionCaseType::AgentRunIncident,
            AttentionCaseStatus::Acknowledged,
        ));
        assert!(AttentionCaseStatus::Acknowledged.can_transition_to(
            AttentionCaseType::AgentRunIncident,
            AttentionCaseStatus::Resolved,
        ));
        assert!(!AttentionCaseStatus::Resolved.can_transition_to(
            AttentionCaseType::AgentRunIncident,
            AttentionCaseStatus::Open,
        ));
    }
}

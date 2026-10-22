#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PhasePlanStatus {
    Draft,
    Active,
    Superseded,
}

impl PhasePlanStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Active => "active",
            Self::Superseded => "superseded",
        }
    }
}

impl TryFrom<&str> for PhasePlanStatus {
    type Error = ();
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "draft" => Ok(Self::Draft),
            "active" => Ok(Self::Active),
            "superseded" => Ok(Self::Superseded),
            _ => Err(()),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProjectPhaseStatus {
    Planned,
    InProgress,
    Review,
    Completed,
    Paused,
    Terminated,
}

impl ProjectPhaseStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Planned => "planned",
            Self::InProgress => "in_progress",
            Self::Review => "review",
            Self::Completed => "completed",
            Self::Paused => "paused",
            Self::Terminated => "terminated",
        }
    }

    #[must_use]
    pub const fn can_transition_to(self, target: Self) -> bool {
        matches!(
            (self, target),
            (
                Self::Planned,
                Self::InProgress | Self::Paused | Self::Terminated
            ) | (
                Self::InProgress,
                Self::Review | Self::Paused | Self::Terminated
            ) | (
                Self::Review,
                Self::Completed | Self::InProgress | Self::Paused | Self::Terminated
            ) | (Self::Paused, Self::InProgress | Self::Terminated)
        )
    }
}

impl TryFrom<&str> for ProjectPhaseStatus {
    type Error = ();
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "planned" => Ok(Self::Planned),
            "in_progress" => Ok(Self::InProgress),
            "review" => Ok(Self::Review),
            "completed" => Ok(Self::Completed),
            "paused" => Ok(Self::Paused),
            "terminated" => Ok(Self::Terminated),
            _ => Err(()),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BusinessTaskStatus {
    Candidate,
    Todo,
    InProgress,
    Review,
    Decision,
    Rework,
    Completed,
    Terminated,
}

impl BusinessTaskStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Candidate => "candidate",
            Self::Todo => "todo",
            Self::InProgress => "in_progress",
            Self::Review => "review",
            Self::Decision => "decision",
            Self::Rework => "rework",
            Self::Completed => "completed",
            Self::Terminated => "terminated",
        }
    }

    #[must_use]
    pub const fn can_transition_to(self, target: Self) -> bool {
        matches!(
            (self, target),
            (Self::Candidate, Self::Todo | Self::Terminated)
                | (
                    Self::Todo | Self::Rework,
                    Self::InProgress | Self::Terminated
                )
                | (Self::InProgress, Self::Review | Self::Terminated)
                | (Self::Todo | Self::InProgress | Self::Review, Self::Decision)
                | (
                    Self::Decision,
                    Self::Todo | Self::InProgress | Self::Review | Self::Terminated
                )
                | (
                    Self::Review,
                    Self::Completed | Self::Rework | Self::Terminated
                )
        )
    }
}

impl TryFrom<&str> for BusinessTaskStatus {
    type Error = ();
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "candidate" => Ok(Self::Candidate),
            "todo" => Ok(Self::Todo),
            "in_progress" => Ok(Self::InProgress),
            "review" => Ok(Self::Review),
            "decision" => Ok(Self::Decision),
            "rework" => Ok(Self::Rework),
            "completed" => Ok(Self::Completed),
            "terminated" => Ok(Self::Terminated),
            _ => Err(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn progress_transitions_require_explicit_review() {
        assert!(BusinessTaskStatus::Review.can_transition_to(BusinessTaskStatus::Completed));
        assert!(!BusinessTaskStatus::InProgress.can_transition_to(BusinessTaskStatus::Completed));
        assert!(!BusinessTaskStatus::Decision.can_transition_to(BusinessTaskStatus::Completed));
        assert!(BusinessTaskStatus::Decision.can_transition_to(BusinessTaskStatus::InProgress));
        assert!(ProjectPhaseStatus::Review.can_transition_to(ProjectPhaseStatus::Completed));
        assert!(!ProjectPhaseStatus::InProgress.can_transition_to(ProjectPhaseStatus::Completed));
        assert!(ProjectPhaseStatus::Paused.can_transition_to(ProjectPhaseStatus::InProgress));
    }
}

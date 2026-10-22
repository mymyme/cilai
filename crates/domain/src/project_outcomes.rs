#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProjectStatus {
    Active,
    Paused,
    Completed,
    Archived,
}

impl ProjectStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Paused => "paused",
            Self::Completed => "completed",
            Self::Archived => "archived",
        }
    }
}

impl TryFrom<&str> for ProjectStatus {
    type Error = ();

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "active" => Ok(Self::Active),
            "paused" => Ok(Self::Paused),
            "completed" => Ok(Self::Completed),
            "archived" => Ok(Self::Archived),
            _ => Err(()),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeliverableStatus {
    Draft,
    Candidate,
    Accepted,
    RevisionNeeded,
}

impl DeliverableStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Candidate => "candidate",
            Self::Accepted => "accepted",
            Self::RevisionNeeded => "revision_needed",
        }
    }
}

impl TryFrom<&str> for DeliverableStatus {
    type Error = ();

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "draft" => Ok(Self::Draft),
            "candidate" => Ok(Self::Candidate),
            "accepted" => Ok(Self::Accepted),
            "revision_needed" => Ok(Self::RevisionNeeded),
            _ => Err(()),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeliverableVersionStatus {
    Candidate,
    Accepted,
    Rejected,
    Superseded,
}

impl DeliverableVersionStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Candidate => "candidate",
            Self::Accepted => "accepted",
            Self::Rejected => "rejected",
            Self::Superseded => "superseded",
        }
    }
}

impl TryFrom<&str> for DeliverableVersionStatus {
    type Error = ();

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "candidate" => Ok(Self::Candidate),
            "accepted" => Ok(Self::Accepted),
            "rejected" => Ok(Self::Rejected),
            "superseded" => Ok(Self::Superseded),
            _ => Err(()),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AcceptanceDecision {
    Accepted,
    Rejected,
}

impl AcceptanceDecision {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Rejected => "rejected",
        }
    }
}

impl TryFrom<&str> for AcceptanceDecision {
    type Error = ();

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "accepted" => Ok(Self::Accepted),
            "rejected" => Ok(Self::Rejected),
            _ => Err(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_outcome_states_have_frozen_storage_values() {
        assert_eq!(ProjectStatus::Active.as_str(), "active");
        assert_eq!(
            DeliverableStatus::RevisionNeeded.as_str(),
            "revision_needed"
        );
        assert_eq!(DeliverableVersionStatus::Superseded.as_str(), "superseded");
        assert_eq!(AcceptanceDecision::Rejected.as_str(), "rejected");
    }
}

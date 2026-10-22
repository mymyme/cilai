#![doc = "`ContextHub` 与 `UI`、存储和操作系统无关的领域模型。"]

mod file_state;
mod project_outcomes;
mod project_progress;
mod work_attention;

pub use file_state::*;
pub use project_outcomes::*;
pub use project_progress::*;
pub use work_attention::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceId(String);

impl WorkspaceId {
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountId(String);

impl AccountId {
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EventId(String);

impl EventId {
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocalWorkspaceStatus {
    Creating,
    Scanning,
    Ready,
    AttentionRequired,
    Unavailable,
    Invalid,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkspaceKind {
    Local,
    CloudTeam,
}

impl WorkspaceKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::CloudTeam => "cloud_team",
        }
    }
}

impl TryFrom<&str> for WorkspaceKind {
    type Error = ();

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "local" => Ok(Self::Local),
            "cloud_team" => Ok(Self::CloudTeam),
            _ => Err(()),
        }
    }
}

impl LocalWorkspaceStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Creating => "creating",
            Self::Scanning => "scanning",
            Self::Ready => "ready",
            Self::AttentionRequired => "attention_required",
            Self::Unavailable => "unavailable",
            Self::Invalid => "invalid",
        }
    }
}

impl TryFrom<&str> for LocalWorkspaceStatus {
    type Error = ();

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "creating" => Ok(Self::Creating),
            "scanning" => Ok(Self::Scanning),
            "ready" => Ok(Self::Ready),
            "attention_required" => Ok(Self::AttentionRequired),
            "unavailable" => Ok(Self::Unavailable),
            "invalid" => Ok(Self::Invalid),
            _ => Err(()),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkspacePlatformBindingState {
    Binding,
    Bound,
    Unauthorized,
    Disconnected,
    Invalid,
}

impl WorkspacePlatformBindingState {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Binding => "binding",
            Self::Bound => "bound",
            Self::Unauthorized => "unauthorized",
            Self::Disconnected => "disconnected",
            Self::Invalid => "invalid",
        }
    }
}

impl TryFrom<&str> for WorkspacePlatformBindingState {
    type Error = ();

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "binding" => Ok(Self::Binding),
            "bound" => Ok(Self::Bound),
            "unauthorized" => Ok(Self::Unauthorized),
            "disconnected" => Ok(Self::Disconnected),
            "invalid" => Ok(Self::Invalid),
            _ => Err(()),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskId(String);

impl TaskId {
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskAttemptId(String);

impl TaskAttemptId {
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TaskState {
    Queued,
    Running,
    WaitingForUser,
    Succeeded,
    Failed,
    Cancelled,
    Interrupted,
    OutcomeUnknown,
}

impl TaskState {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::WaitingForUser => "waiting_for_user",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Interrupted => "interrupted",
            Self::OutcomeUnknown => "outcome_unknown",
        }
    }

    #[must_use]
    pub const fn can_transition_to(self, target: Self) -> bool {
        matches!(
            (self, target),
            (Self::Queued, Self::Running | Self::Cancelled)
                | (
                    Self::Running,
                    Self::WaitingForUser
                        | Self::Succeeded
                        | Self::Failed
                        | Self::Cancelled
                        | Self::Interrupted
                        | Self::OutcomeUnknown
                )
                | (
                    Self::WaitingForUser,
                    Self::Running | Self::Failed | Self::Cancelled | Self::Interrupted
                )
                | (Self::Interrupted, Self::Running)
                | (Self::OutcomeUnknown, Self::Succeeded | Self::Failed)
        )
    }

    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }
}

impl TryFrom<&str> for TaskState {
    type Error = ();

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "queued" => Ok(Self::Queued),
            "running" => Ok(Self::Running),
            "waiting_for_user" => Ok(Self::WaitingForUser),
            "succeeded" => Ok(Self::Succeeded),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            "interrupted" => Ok(Self::Interrupted),
            "outcome_unknown" => Ok(Self::OutcomeUnknown),
            _ => Err(()),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TaskAttemptState {
    Running,
    Succeeded,
    Failed,
    Interrupted,
}

impl TaskAttemptState {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Interrupted => "interrupted",
        }
    }
}

impl TryFrom<&str> for TaskAttemptState {
    type Error = ();

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "running" => Ok(Self::Running),
            "succeeded" => Ok(Self::Succeeded),
            "failed" => Ok(Self::Failed),
            "interrupted" => Ok(Self::Interrupted),
            _ => Err(()),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PathCaseSensitivity {
    Sensitive,
    Insensitive,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkspacePathError {
    Empty,
    Absolute,
    ParentTraversal,
    NonUnicode,
    NullByte,
    NonPortableSeparator,
}

impl std::fmt::Display for WorkspacePathError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "workspace path is empty",
            Self::Absolute => "workspace path must be relative",
            Self::ParentTraversal => "workspace path contains parent traversal",
            Self::NonUnicode => "workspace path is not valid Unicode",
            Self::NullByte => "workspace path contains a null byte",
            Self::NonPortableSeparator => {
                "native workspace path contains a non-portable separator character"
            }
        })
    }
}

impl std::error::Error for WorkspacePathError {}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct WorkspaceRelativePath(String);

impl WorkspaceRelativePath {
    /// 解析来自协议或持久化层的平台无关路径；输入可使用 `/` 或 Windows `\` 分隔符。
    ///
    /// # Errors
    ///
    /// 路径为空、为绝对路径、包含父级跳转或空字节时返回错误。
    pub fn parse(input: &str) -> Result<Self, WorkspacePathError> {
        if input.contains('\0') {
            return Err(WorkspacePathError::NullByte);
        }
        let portable = input.replace('\\', "/");
        if portable.starts_with('/') || has_windows_prefix(&portable) {
            return Err(WorkspacePathError::Absolute);
        }
        Self::from_segments(portable.split('/'))
    }

    /// 从当前系统的相对 `Path` 建立平台无关路径。
    ///
    /// # Errors
    ///
    /// 路径为绝对路径、包含父级跳转、非 Unicode 或无法跨平台表达的分隔符时返回错误。
    pub fn from_native(path: &std::path::Path) -> Result<Self, WorkspacePathError> {
        let mut segments = Vec::new();
        for component in path.components() {
            match component {
                std::path::Component::CurDir => {}
                std::path::Component::ParentDir => {
                    return Err(WorkspacePathError::ParentTraversal);
                }
                std::path::Component::Normal(segment) => {
                    let segment = segment.to_str().ok_or(WorkspacePathError::NonUnicode)?;
                    if segment.contains('\\') {
                        return Err(WorkspacePathError::NonPortableSeparator);
                    }
                    if segment.contains('\0') {
                        return Err(WorkspacePathError::NullByte);
                    }
                    segments.push(segment.to_owned());
                }
                std::path::Component::RootDir | std::path::Component::Prefix(_) => {
                    return Err(WorkspacePathError::Absolute);
                }
            }
        }
        if segments.is_empty() {
            return Err(WorkspacePathError::Empty);
        }
        Ok(Self(segments.join("/")))
    }

    fn from_segments<'a>(
        segments: impl IntoIterator<Item = &'a str>,
    ) -> Result<Self, WorkspacePathError> {
        let mut normalized = Vec::new();
        for segment in segments {
            match segment {
                "" | "." => {}
                ".." => return Err(WorkspacePathError::ParentTraversal),
                value => normalized.push(value),
            }
        }
        if normalized.is_empty() {
            return Err(WorkspacePathError::Empty);
        }
        Ok(Self(normalized.join("/")))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn segments(&self) -> impl Iterator<Item = &str> {
        self.0.split('/')
    }

    #[must_use]
    pub fn comparison_key(&self, sensitivity: PathCaseSensitivity) -> String {
        match sensitivity {
            PathCaseSensitivity::Sensitive => self.0.clone(),
            PathCaseSensitivity::Insensitive => {
                self.0.chars().flat_map(char::to_lowercase).collect()
            }
        }
    }
}

impl std::fmt::Display for WorkspaceRelativePath {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

fn has_windows_prefix(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

#[cfg(test)]
mod tests {
    use super::{PathCaseSensitivity, TaskState, WorkspacePathError, WorkspaceRelativePath};

    #[test]
    fn task_state_machine_only_allows_frozen_transitions() {
        assert!(TaskState::Queued.can_transition_to(TaskState::Running));
        assert!(TaskState::Running.can_transition_to(TaskState::WaitingForUser));
        assert!(TaskState::WaitingForUser.can_transition_to(TaskState::Running));
        assert!(TaskState::Interrupted.can_transition_to(TaskState::Running));
        assert!(TaskState::OutcomeUnknown.can_transition_to(TaskState::Succeeded));
        assert!(!TaskState::Succeeded.can_transition_to(TaskState::Running));
        assert!(!TaskState::OutcomeUnknown.can_transition_to(TaskState::Running));
    }

    #[test]
    fn only_definitive_completion_states_are_terminal() {
        assert!(TaskState::Succeeded.is_terminal());
        assert!(TaskState::Failed.is_terminal());
        assert!(TaskState::Cancelled.is_terminal());
        assert!(!TaskState::Interrupted.is_terminal());
        assert!(!TaskState::OutcomeUnknown.is_terminal());
    }

    #[test]
    fn workspace_path_normalizes_portable_separators_and_current_segments() {
        let path = WorkspaceRelativePath::parse("./docs\\design//architecture.md/").unwrap();
        assert_eq!(path.as_str(), "docs/design/architecture.md");
        assert_eq!(
            path.segments().collect::<Vec<_>>(),
            vec!["docs", "design", "architecture.md"]
        );
        assert_eq!(
            path.comparison_key(PathCaseSensitivity::Insensitive),
            "docs/design/architecture.md"
        );
    }

    #[test]
    fn workspace_path_rejects_absolute_parent_and_empty_inputs() {
        for (input, expected) in [
            ("/etc/passwd", WorkspacePathError::Absolute),
            ("C:\\Windows", WorkspacePathError::Absolute),
            ("../outside", WorkspacePathError::ParentTraversal),
            ("docs/../../outside", WorkspacePathError::ParentTraversal),
            ("./", WorkspacePathError::Empty),
        ] {
            assert_eq!(WorkspaceRelativePath::parse(input), Err(expected));
        }
    }

    #[test]
    fn comparison_key_keeps_display_case_separate_from_collision_semantics() {
        let path = WorkspaceRelativePath::parse("Docs/ReadMe.md").unwrap();
        assert_eq!(
            path.comparison_key(PathCaseSensitivity::Sensitive),
            "Docs/ReadMe.md"
        );
        assert_eq!(
            path.comparison_key(PathCaseSensitivity::Insensitive),
            "docs/readme.md"
        );
        assert_eq!(path.as_str(), "Docs/ReadMe.md");
    }

    #[test]
    fn native_workspace_path_uses_the_same_traversal_boundary() {
        let path =
            WorkspaceRelativePath::from_native(std::path::Path::new("docs/./readme.md")).unwrap();
        assert_eq!(path.as_str(), "docs/readme.md");
        assert_eq!(
            WorkspaceRelativePath::from_native(std::path::Path::new("docs/../outside")),
            Err(WorkspacePathError::ParentTraversal)
        );
    }

    #[test]
    fn workspace_path_preserves_unicode_code_points_without_normalizing() {
        let composed = WorkspaceRelativePath::parse("docs/\u{e9}.md").unwrap();
        let decomposed = WorkspaceRelativePath::parse("docs/e\u{301}.md").unwrap();

        assert_eq!(composed.as_str(), "docs/\u{e9}.md");
        assert_eq!(decomposed.as_str(), "docs/e\u{301}.md");
        assert_ne!(composed, decomposed);
        assert_ne!(
            composed.comparison_key(PathCaseSensitivity::Insensitive),
            decomposed.comparison_key(PathCaseSensitivity::Insensitive)
        );
    }

    #[cfg(unix)]
    #[test]
    fn native_non_unicode_path_is_rejected_before_entering_the_domain() {
        use std::os::unix::ffi::OsStringExt as _;

        let native = std::ffi::OsString::from_vec(vec![b'd', b'o', b'c', b's', b'/', 0xff]);
        assert_eq!(
            WorkspaceRelativePath::from_native(std::path::Path::new(&native)),
            Err(WorkspacePathError::NonUnicode)
        );
    }
}

//! Private per-attempt evidence. Only typed, bounded summaries are exported.
use super::super::{DaclFlag, RepairAttemptDiagnostic, RepairPhase, RepairScope, SecurityPart};
use super::*;
use sha2::{Digest as _, Sha256};
use std::io::{Read as _, Seek as _, SeekFrom};

const LIMIT: u64 = 256 * 1024;

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct RepairJournal {
    schema_version: u32,
    environment: String,
    pub(super) summary: RepairAttemptDiagnostic,
    // These are retained locally only; Diagnostics never serializes them.
    target_path: Option<PathBuf>,
    expected_descriptor: Option<String>,
    observed_descriptor: Option<String>,
    #[serde(default)]
    inherited_source_conversions: Vec<usize>,
}

impl RepairJournal {
    pub(super) fn new(request: &Request) -> io::Result<Self> {
        let mut executable = File::open(std::env::current_exe()?)?;
        let mut hash = Sha256::new();
        let mut bytes = [0; 16 * 1024];
        loop {
            let count = executable.read(&mut bytes)?;
            if count == 0 {
                break;
            }
            hash.update(&bytes[..count]);
        }
        Ok(Self {
            schema_version: 1,
            environment: request.plan.environment.clone(),
            summary: RepairAttemptDiagnostic {
                operation_id: request.backup_name.trim_end_matches(".json").to_owned(),
                at_epoch_seconds: now(),
                component_sha256: format!("{:x}", hash.finalize()),
                phase: RepairPhase::Prepared,
                target_index: None,
                target_scope: None,
                previous_applied_index: None,
                changed_parts: Vec::new(),
                dacl_flags_before: Vec::new(),
                dacl_flags_after: Vec::new(),
                os_error: None,
                rollback_verified: None,
            },
            target_path: None,
            expected_descriptor: None,
            observed_descriptor: None,
            inherited_source_conversions: Vec::new(),
        })
    }

    pub(super) fn target(&mut self, index: usize, target: &Target) {
        self.summary.phase = RepairPhase::Compare;
        self.summary.target_index = Some(index);
        self.summary.target_scope = Some(scope(target));
        self.target_path = Some(target.path.clone());
    }

    pub(super) fn difference(&mut self, expected: &str, observed: &str) {
        self.summary.changed_parts = differences(expected, observed);
        self.summary.dacl_flags_before = flags(expected);
        self.summary.dacl_flags_after = flags(observed);
        self.expected_descriptor = Some(expected.to_owned());
        self.observed_descriptor = Some(observed.to_owned());
    }

    pub(super) fn failed(&mut self, error: &io::Error, rollback_verified: bool) {
        self.summary.os_error = error.raw_os_error();
        self.summary.rollback_verified = Some(rollback_verified);
    }

    pub(super) fn inherited_source_conversion(&mut self, index: usize) {
        self.inherited_source_conversions.push(index);
    }
}

fn scope(target: &Target) -> RepairScope {
    if target.runtime {
        return RepairScope::RuntimeDirectory;
    }
    if target
        .path
        .parent()
        .and_then(Path::file_name)
        .is_some_and(|name| {
            ["cilai-harness", "cilai-application-harness"]
                .iter()
                .any(|home| name == OsStr::new(home))
        })
    {
        return RepairScope::RuntimeDatabase;
    }
    if target.path.parent().and_then(Path::file_name) == Some(OsStr::new("profiles"))
        && target.directory
    {
        return RepairScope::Profile;
    }
    match target.path.file_name().and_then(OsStr::to_str) {
        Some("cn.cilai.personal.workbench") => RepairScope::ApplicationData,
        Some("business") => RepairScope::Environment,
        Some("profiles") => RepairScope::Profiles,
        _ if target.directory => RepairScope::StartupDirectory,
        _ => RepairScope::StartupFile,
    }
}

fn section(sddl: &str, prefix: &str) -> String {
    let Some((_, rest)) = sddl.split_once(prefix) else {
        return String::new();
    };
    let end = ["O:", "G:", "D:", "S:"]
        .iter()
        .filter_map(|next| rest.find(next))
        .min()
        .unwrap_or(rest.len());
    rest[..end].to_owned()
}

fn differences(before: &str, after: &str) -> Vec<SecurityPart> {
    let mut parts = Vec::new();
    if owner(before).ok() != owner(after).ok() {
        parts.push(SecurityPart::Owner);
    }
    if section(before, "G:") != section(after, "G:") {
        parts.push(SecurityPart::Group);
    }
    let before_dacl = section(before, "D:");
    let after_dacl = section(after, "D:");
    let split = |dacl: &str| -> (String, String) {
        let index = dacl.find('(').unwrap_or(dacl.len());
        (dacl[..index].to_owned(), dacl[index..].to_owned())
    };
    let (before_flags, before_entries) = split(&before_dacl);
    let (after_flags, after_entries) = split(&after_dacl);
    if before_flags != after_flags {
        parts.push(SecurityPart::DaclFlags);
    }
    if before_entries != after_entries {
        parts.push(SecurityPart::DaclEntries);
    }
    if parts.is_empty() && before != after {
        parts.push(SecurityPart::Representation);
    }
    parts
}

fn flags(sddl: &str) -> Vec<DaclFlag> {
    let dacl = section(sddl, "D:");
    let flags = dacl.split('(').next().unwrap_or_default();
    [
        ("P", DaclFlag::Protected),
        ("AI", DaclFlag::AutoInherited),
        ("AR", DaclFlag::AutoInheritRequested),
    ]
    .into_iter()
    .filter_map(|(marker, flag)| flags.contains(marker).then_some(flag))
    .collect()
}

pub(super) fn path(request: &Request) -> io::Result<PathBuf> {
    Ok(backup_path(request)?.with_extension("repair.json"))
}

pub(super) fn open_for_write(request: &Request) -> io::Result<File> {
    let file = open(&path(request)?, false, FILE_GENERIC_WRITE | READ_CONTROL)?;
    if identity(&file)? != request.journal_identity
        || owner(&snapshot(&file)?)? != request.plan.user_sid
    {
        return Err(conflict(Conflict::BackupChanged));
    }
    Ok(file)
}

pub(super) fn save(file: &mut File, journal: &RepairJournal) -> io::Result<()> {
    let bytes = serde_json::to_vec_pretty(journal).map_err(io::Error::other)?;
    if bytes.len() as u64 > LIMIT {
        return Err(error(UNSAFE_OBJECT));
    }
    file.seek(SeekFrom::Start(0))?;
    file.write_all(&bytes)?;
    file.set_len(bytes.len() as u64)?;
    file.sync_all()
}

fn operation_id(name: &str) -> Option<&str> {
    let id = name.strip_suffix(".repair.json")?;
    (id.len() == 36
        && id.bytes().enumerate().all(|(index, byte)| {
            if [8, 13, 18, 23].contains(&index) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        }))
    .then_some(id)
}

pub(super) fn recent(plan: &RepairPlan) -> io::Result<Vec<RepairAttemptDiagnostic>> {
    let directory = layout::recovery(Path::new(&plan.local_data));
    let _pins = match pin_chain(&directory) {
        Ok(pins) => pins,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    crate::security::verify_current_user_private_file(&directory)?;
    let mut candidates = Vec::new();
    for entry in std::fs::read_dir(&directory)?.take(2048) {
        let entry = entry?;
        let name = entry.file_name();
        let Some(id) = name.to_str().and_then(operation_id) else {
            continue;
        };
        let file = open(&entry.path(), false, FILE_GENERIC_READ | READ_CONTROL)?;
        if owner(&snapshot(&file)?)? != plan.user_sid {
            return Err(error(FOREIGN_OWNER));
        }
        let metadata = file.metadata()?;
        if metadata.len() == 0 {
            continue;
        }
        if metadata.len() > LIMIT {
            return Err(error(UNSAFE_OBJECT));
        }
        candidates.push((metadata.modified()?, id.to_owned(), file));
    }
    candidates.sort_by_key(|candidate| std::cmp::Reverse(candidate.0));
    let mut results = Vec::new();
    for (_, id, file) in candidates.into_iter().take(3) {
        let mut bytes = Vec::new();
        file.take(LIMIT + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > LIMIT {
            return Err(error(UNSAFE_OBJECT));
        }
        let journal: RepairJournal = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
        let summary = &journal.summary;
        if journal.schema_version != 1
            || summary.operation_id != id
            || !summary
                .component_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            || summary.component_sha256.len() != 64
            || summary.target_index.is_some_and(|index| index > 128)
            || summary
                .previous_applied_index
                .is_some_and(|index| index > 128)
            || summary.changed_parts.len() > 5
            || summary.dacl_flags_before.len() > 3
            || summary.dacl_flags_after.len() > 3
            || summary.at_epoch_seconds > now() + 300
        {
            return Err(error(UNSAFE_OBJECT));
        }
        if journal.environment == plan.environment && summary.phase != RepairPhase::Complete {
            results.push(journal.summary);
        }
    }
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn distinguishes_ownership_inheritance_flags_and_access_rules() {
        let before = "O:BAG:SYD:AI(A;;FA;;;SY)";
        assert_eq!(
            differences(before, "O:BAG:SYD:PAI(A;;FA;;;SY)"),
            vec![SecurityPart::DaclFlags]
        );
        assert_eq!(
            differences(before, "O:BAG:SYD:AI(A;;FR;;;SY)"),
            vec![SecurityPart::DaclEntries]
        );
        assert_eq!(
            differences(before, "O:SYG:SYD:AI(A;;FA;;;SY)"),
            vec![SecurityPart::Owner]
        );
        assert_eq!(
            differences(before, "O:BAG:BAD:AI(A;;FA;;;SY)"),
            vec![SecurityPart::Group]
        );
        assert_eq!(
            differences(before, "O:S-1-5-32-544G:SYD:AI(A;;FA;;;SY)"),
            vec![SecurityPart::Representation]
        );
    }
}

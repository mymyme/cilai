use super::*;
use contexthub_persistence_sqlite::{
    PersonalCapabilityVersion, PersonalPackageImportCommand, PersonalPackageImportRecord,
};
use contexthub_protocol::{
    PersonalPackageImportCancelParams, PersonalPackageImportCommitParams,
    PersonalPackageImportCommitResult, PersonalPackageImportListParams,
    PersonalPackageImportParams, PersonalPackageImportPrepareParams, PersonalPackageImportPreview,
    PersonalPackageImportSource, PersonalPackageImportState, PersonalPackageImportTarget,
};
use serde::Serialize;
use serde_json::Value;

#[cfg(test)]
mod tests;

const MAX_ARCHIVE_BYTES: usize = 2 * 1024 * 1024;
const IMPORT_ROOT: &str = "personal-package-imports";

fn invalid() -> ServiceError {
    ServiceError::new("personal_package_import_invalid")
}
fn hash(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}
fn fingerprint(value: &impl Serialize) -> Result<String, ServiceError> {
    serde_json::to_vec(value)
        .map(|bytes| hash(&bytes))
        .map_err(|_| invalid())
}
fn validate_id(id: &str) -> Result<(), ServiceError> {
    if id.strip_prefix("import_").is_some_and(|s| {
        s.len() == 32
            && s.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    }) {
        Ok(())
    } else {
        Err(ServiceError::new("personal_package_import_id_invalid"))
    }
}
pub(super) fn read_archive(path: &Path) -> Result<Vec<u8>, ServiceError> {
    let file = contexthub_platform_adapter::filesystem::open_regular_file_no_follow(path, false)
        .map_err(|_| ServiceError::new("personal_package_import_archive_unavailable"))?;
    if file.metadata().map_err(|_| invalid())?.len() > MAX_ARCHIVE_BYTES as u64 {
        return Err(ServiceError::new("personal_package_import_too_large"));
    }
    let mut bytes = Vec::new();
    file.take(MAX_ARCHIVE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ServiceError::new("personal_package_import_archive_unavailable"))?;
    if bytes.len() > MAX_ARCHIVE_BYTES {
        return Err(ServiceError::new("personal_package_import_too_large"));
    }
    Ok(bytes)
}

pub(super) fn decode_archive(bytes: &[u8]) -> Result<BTreeMap<String, Vec<u8>>, ServiceError> {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).map_err(|_| invalid())?;
    if archive.is_empty() || archive.len() > MAX_SOURCE_FILES + 3 {
        return Err(invalid());
    }
    let mut files = BTreeMap::new();
    let mut folded = HashSet::new();
    let mut total = 0usize;
    for index in 0..archive.len() {
        let entry = archive.by_index(index).map_err(|_| invalid())?;
        let name = entry.name().map_err(|_| invalid())?.into_owned();
        if entry.is_dir()
            || entry.encrypted()
            || name.contains('\\')
            || entry
                .unix_mode()
                .is_some_and(|mode| !matches!(mode & 0o170_000, 0 | 0o100_000))
            || entry.size() > MAX_SOURCE_FILE_BYTES as u64
            || !folded.insert(name.to_ascii_lowercase())
        {
            return Err(invalid());
        }
        if entry.enclosed_name().is_none()
            || name.starts_with('/')
            || name.contains(':')
            || name
                .split('/')
                .any(|s| s.is_empty() || matches!(s, "." | ".."))
        {
            return Err(invalid());
        }
        let mut content = Vec::new();
        entry
            .take(MAX_SOURCE_FILE_BYTES as u64 + 1)
            .read_to_end(&mut content)
            .map_err(|_| invalid())?;
        total = total
            .checked_add(content.len())
            .filter(|n| *n <= MAX_ARCHIVE_BYTES)
            .ok_or_else(invalid)?;
        if content.len() > MAX_SOURCE_FILE_BYTES {
            return Err(invalid());
        }
        files.insert(name, content);
    }
    Ok(files)
}

struct Decoded {
    source: PersonalPackageImportSource,
    permissions: Vec<String>,
    files: Vec<CapabilityCreationSourceFile>,
}

fn has_migration(value: &Value) -> bool {
    match value {
        Value::Object(object) => object.iter().any(|(key, value)| {
            matches!(
                key.as_str(),
                "migrations"
                    | "migration"
                    | "dataMigration"
                    | "dataMigrations"
                    | "dataVersion"
                    | "schemaMigrations"
            ) || has_migration(value)
        }),
        Value::Array(values) => values.iter().any(has_migration),
        _ => false,
    }
}

fn manifest(files: &BTreeMap<String, Vec<u8>>, path: &str) -> Result<Value, ServiceError> {
    let value: Value =
        serde_json::from_slice(files.get(path).ok_or_else(invalid)?).map_err(|_| invalid())?;
    if value.as_object().is_none() {
        return Err(invalid());
    }
    if has_migration(&value) {
        return Err(ServiceError::new(
            "personal_package_import_data_migration_unsupported",
        ));
    }
    Ok(value)
}
fn text(value: &Value, key: &str) -> Result<String, ServiceError> {
    value[key].as_str().map(str::to_owned).ok_or_else(invalid)
}
fn valid_version(version: &str) -> bool {
    let fields: Vec<_> = version.split('.').collect();
    fields.len() == 3
        && fields
            .iter()
            .all(|field| field.parse::<u32>().is_ok_and(|n| n.to_string() == *field))
}

fn source_permissions(app: &Value, target: &str) -> Result<Vec<String>, ServiceError> {
    let permissions: Vec<String> =
        serde_json::from_value(app["requestedPermissions"].clone()).map_err(|_| invalid())?;
    pages::validate_permissions(target, &permissions)?;
    Ok(permissions)
}

fn inspect(bytes: &[u8]) -> Result<Decoded, ServiceError> {
    let package = decode_archive(bytes)?;
    let plugin = manifest(&package, "generated/plugin-manifest.json")?;
    let app = manifest(&package, "generated/personal-application.json")?;
    let plugin_id = text(&plugin, "pluginId")?;
    let foreign_id = format!(
        "capability_{}",
        plugin_id.strip_prefix("personal.").ok_or_else(invalid)?
    );
    validate_project_id(&foreign_id)?;
    let version = text(&plugin, "version")?;
    if !valid_version(&version) {
        return Err(ServiceError::new("personal_package_import_version_invalid"));
    }
    let title = text(&plugin, "name")?;
    let summary = text(&plugin, "summary")?;
    validate_text(&title, 160, "personal_package_import_invalid")?;
    if summary.trim().is_empty() || summary.chars().count() > 200 || summary.contains('\0') {
        return Err(invalid());
    }
    let target = match (
        app["entrypoint"].is_string(),
        package.contains_key("generated/local-runtime.json"),
    ) {
        (true, false) => "web_application",
        (false, true) => "local_mcp",
        (true, true) => "web_mcp_application",
        _ => return Err(invalid()),
    };
    if target != "web_application" {
        manifest(&package, "generated/local-runtime.json")?;
    }
    let permissions = source_permissions(&app, target)?;
    let source: BTreeMap<_, _> = package
        .iter()
        .filter_map(|(p, c)| p.strip_prefix("source/").map(|p| (p.to_owned(), c.clone())))
        .collect();
    let inputs = source
        .iter()
        .map(|(path, c)| {
            Ok(CapabilityCreationSourceFile {
                path: path.clone(),
                content: String::from_utf8(c.clone()).map_err(|_| invalid())?,
            })
        })
        .collect::<Result<Vec<_>, ServiceError>>()?;
    validate_source_files(target, &inputs)?;
    if inputs.is_empty() {
        return Err(invalid());
    }
    let digest = format!("sha256:{}", digest_files(&source));
    let mut expected = generate_personal_manifests(
        &foreign_id,
        &title,
        &summary,
        target,
        &permissions,
        &digest,
        &version,
        pages::from_files(target, &permissions, &inputs)?.as_ref(),
    )?;
    expected.extend(
        source
            .iter()
            .map(|(p, c)| (format!("source/{p}"), c.clone())),
    );
    if expected != package {
        return Err(ServiceError::new(
            "personal_package_import_manifest_invalid",
        ));
    }
    let mut normalized = permissions;
    normalized.sort();
    Ok(Decoded {
        source: PersonalPackageImportSource {
            page_ai: pages::from_files(target, &normalized, &inputs)?,
            authenticated: false,
            format_version: 1,
            archive_digest: hash(bytes),
            source_digest: digest,
            plugin_id,
            version,
            title,
            summary,
            target_type: target.into(),
            source_file_count: u32::try_from(source.len()).map_err(|_| invalid())?,
            archive_bytes: bytes.len() as u64,
            source_bytes: source.values().map(|c| c.len() as u64).sum(),
            unpacked_bytes: package.values().map(|c| c.len() as u64).sum(),
            full_business_goal_present: false,
            acceptance_examples_present: false,
        },
        permissions: normalized,
        files: inputs,
    })
}

fn preview_digest(preview: &PersonalPackageImportPreview) -> Result<String, ServiceError> {
    fingerprint(
        &json!({"importId":preview.import_id,"profileId":preview.profile_id,"preparedAt":preview.prepared_at,"source":preview.source,"target":preview.target,"localCreationProjectId":preview.local_creation_project_id,"expectedRevision":preview.expected_revision,"installedDigest":preview.installed_digest,"localVersion":preview.local_version,"localPackageDigest":preview.local_package_digest,"requestedPermissions":preview.requested_permissions,"addedPermissions":preview.added_permissions}),
    )
}

fn sync_tree(path: &Path) -> Result<(), ServiceError> {
    reject_symlink(path)?;
    for entry in fs::read_dir(path).map_err(|_| invalid())? {
        let entry = entry.map_err(|_| invalid())?;
        let kind = entry.file_type().map_err(|_| invalid())?;
        if kind.is_symlink() {
            return Err(invalid());
        }
        if kind.is_dir() {
            sync_tree(&entry.path())?;
        } else if !kind.is_file() {
            return Err(invalid());
        }
    }
    contexthub_platform_adapter::filesystem::sync_directory(path)
        .map_err(|_| ServiceError::new("personal_package_import_sync_failed"))
}

fn sync_ancestors(profile: &Path, path: &Path) -> Result<(), ServiceError> {
    check_path(profile, path)?;
    let mut current = path;
    loop {
        contexthub_platform_adapter::filesystem::sync_directory(current)
            .map_err(|_| ServiceError::new("personal_package_import_sync_failed"))?;
        if current == profile {
            return Ok(());
        }
        current = current.parent().ok_or_else(invalid)?;
    }
}

impl AppServerServices {
    fn package_import_profile(&self) -> Result<String, ServiceError> {
        self.personal_profile_id()
            .ok_or_else(|| ServiceError::new("personal_package_import_profile_not_ready"))
    }
    fn package_import_root(&self, id: &str) -> Result<PathBuf, ServiceError> {
        validate_id(id)?;
        let profile = self
            .data_dir
            .as_ref()
            .ok_or_else(|| ServiceError::new("data_dir_unavailable"))?;
        let root = profile.join(IMPORT_ROOT);
        reject_symlink(&root)?;
        let path = root.join(id);
        reject_symlink(&path)?;
        Ok(path)
    }
    fn import_record(&self, id: &str) -> Result<PersonalPackageImportRecord, ServiceError> {
        validate_id(id)?;
        let profile = self.package_import_profile()?;
        let record = self
            .persistence()?
            .personal_package_import(&profile, id)
            .map_err(persistence_error)?
            .ok_or_else(|| ServiceError::new("personal_package_import_not_found"))?;
        validate_record(&record, &profile)?;
        Ok(record)
    }

    pub(crate) fn personal_package_import_get(
        &mut self,
        p: &PersonalPackageImportParams,
        _: i64,
    ) -> Result<PersonalPackageImportPreview, ServiceError> {
        Ok(self.import_record(&p.import_id)?.preview)
    }
    pub(crate) fn personal_package_import_list(
        &mut self,
        _: &PersonalPackageImportListParams,
        _: i64,
    ) -> Result<Vec<PersonalPackageImportPreview>, ServiceError> {
        let profile = self.package_import_profile()?;
        self.persistence()?
            .personal_package_imports(&profile)
            .map_err(persistence_error)?
            .into_iter()
            .map(|r| {
                validate_record(&r, &profile)?;
                Ok(r.preview)
            })
            .collect()
    }
    pub(crate) fn personal_package_import_cancel(
        &mut self,
        p: &PersonalPackageImportCancelParams,
        now: i64,
    ) -> Result<PersonalPackageImportPreview, ServiceError> {
        self.import_record(&p.import_id)?;
        let profile = self.package_import_profile()?;
        Ok(self
            .persistence_mut()?
            .cancel_personal_package_import(&profile, &p.import_id, &p.preview_digest, now)
            .map_err(persistence_error)?
            .preview)
    }
}

fn validate_record(
    record: &PersonalPackageImportRecord,
    profile: &str,
) -> Result<(), ServiceError> {
    validate_id(&record.preview.import_id)?;
    validate_project_id(&record.preview.local_creation_project_id)?;
    if record.preview.profile_id != profile
        || record.candidate_relative_path
            != format!("{IMPORT_ROOT}/{}/candidate", record.preview.import_id)
        || preview_digest(&record.preview)? != record.preview.preview_digest
        || record.preview.source.authenticated
        || record.preview.source.format_version != 1
    {
        return Err(ServiceError::new("personal_package_import_record_invalid"));
    }
    Ok(())
}

struct TargetState {
    id: String,
    expected_revision: Option<i64>,
    installed_digest: Option<String>,
    version: String,
    previous_permissions: Vec<String>,
    business_goal: Option<String>,
    previous_page_ai: Option<contexthub_protocol::PersonalPageAiManifest>,
}

impl AppServerServices {
    fn import_target(
        &self,
        target: &PersonalPackageImportTarget,
        target_type: &str,
    ) -> Result<TargetState, ServiceError> {
        match target {
            PersonalPackageImportTarget::New {} => Ok(TargetState {
                id: format!("capability_{}", Uuid::new_v4().simple()),
                expected_revision: None,
                installed_digest: None,
                version: "0.1.0".into(),
                previous_permissions: vec![],
                business_goal: None,
                previous_page_ai: None,
            }),
            PersonalPackageImportTarget::Upgrade {
                creation_project_id,
                expected_revision,
            } => {
                validate_project_id(creation_project_id)?;
                self.ensure_personal_capability(creation_project_id)?;
                let project = self
                    .persistence()?
                    .capability_creation_project(creation_project_id)
                    .map_err(persistence_error)?
                    .ok_or_else(|| ServiceError::new("capability_creation_project_not_found"))?;
                let installed = self
                    .persistence()?
                    .personal_capability_installation(creation_project_id)
                    .map_err(persistence_error)?
                    .ok_or_else(|| {
                        ServiceError::new("personal_package_import_target_not_installed")
                    })?;
                if *expected_revision < 1
                    || project.state_revision != *expected_revision
                    || project.status != "installed"
                    || project.target_type != target_type
                    || project.package_digest.as_deref() != Some(installed.package_digest.as_str())
                {
                    return Err(ServiceError::new("personal_package_import_target_changed"));
                }
                let versions = self
                    .persistence()?
                    .personal_capability_versions(creation_project_id)
                    .map_err(persistence_error)?;
                Ok(TargetState {
                    previous_page_ai: self.personal_page_manifest(&project)?,
                    id: creation_project_id.clone(),
                    expected_revision: Some(*expected_revision),
                    installed_digest: Some(installed.package_digest),
                    version: format!("0.1.{}", versions.len() + 1),
                    previous_permissions: serde_json::from_str(&project.requested_permissions_json)
                        .map_err(|_| invalid())?,
                    business_goal: Some(project.business_goal),
                })
            }
        }
    }

    pub(crate) fn personal_package_import_prepare(
        &mut self,
        p: &PersonalPackageImportPrepareParams,
        now: i64,
    ) -> Result<PersonalPackageImportPreview, ServiceError> {
        validate_text(&p.client_operation_id, 160, "client_operation_id_invalid")?;
        let profile = self.package_import_profile()?;
        let request = fingerprint(p)?;
        if let Some(record) = self
            .persistence()?
            .personal_package_import_by_operation(&profile, &p.client_operation_id)
            .map_err(persistence_error)?
        {
            validate_record(&record, &profile)?;
            if record.prepare_fingerprint != request {
                return Err(ServiceError::new("operation_id_reused"));
            }
            return Ok(record.preview);
        }
        if !Path::new(&p.archive_path).is_absolute() || p.archive_path.contains('\0') {
            return Err(ServiceError::new(
                "personal_package_import_archive_path_invalid",
            ));
        }
        let bytes = read_archive(Path::new(&p.archive_path))?;
        let decoded = inspect(&bytes)?;
        let target = self.import_target(&p.target, &decoded.source.target_type)?;
        let id = format!("import_{}", Uuid::new_v4().simple());
        let root = self.package_import_root(&id)?;
        let parent = root.parent().ok_or_else(invalid)?;
        fs::create_dir_all(parent)
            .map_err(|_| ServiceError::new("personal_package_import_write_failed"))?;
        reject_symlink(parent)?;
        fs::create_dir(&root)
            .map_err(|_| ServiceError::new("personal_package_import_write_failed"))?;
        write_new_file(
            &root.join("archive.zip"),
            &bytes,
            "personal_package_import_write_failed",
        )?;
        let candidate = root.join("candidate");
        fs::create_dir(&candidate)
            .map_err(|_| ServiceError::new("personal_package_import_write_failed"))?;
        let built = materialize_source_package(
            &candidate,
            &target.id,
            &decoded.source.title,
            target
                .business_goal
                .as_deref()
                .unwrap_or(&decoded.source.summary),
            &decoded.source.target_type,
            &decoded.permissions,
            &decoded.files,
            &target.version,
        )?;
        sync_tree(&root)?;
        sync_ancestors(self.data_dir.as_deref().ok_or_else(invalid)?, parent)?;
        let mut preview = PersonalPackageImportPreview {
            installed_page_ai: target.previous_page_ai,
            import_id: id.clone(),
            profile_id: profile,
            state: PersonalPackageImportState::Prepared,
            prepared_at: now,
            updated_at: now,
            preview_digest: String::new(),
            source: decoded.source,
            target: p.target.clone(),
            local_creation_project_id: target.id,
            expected_revision: target.expected_revision,
            installed_digest: target.installed_digest,
            local_version: target.version,
            local_package_digest: built.package_digest,
            added_permissions: decoded
                .permissions
                .iter()
                .filter(|permission| !target.previous_permissions.contains(permission))
                .cloned()
                .collect(),
            requested_permissions: decoded.permissions,
        };
        preview.preview_digest = preview_digest(&preview)?;
        let record = PersonalPackageImportRecord {
            preview,
            prepare_operation_id: p.client_operation_id.clone(),
            prepare_fingerprint: request,
            candidate_relative_path: format!("{IMPORT_ROOT}/{id}/candidate"),
            commit_operation_id: None,
            commit_fingerprint: None,
            result: None,
        };
        Ok(self
            .persistence_mut()?
            .prepare_personal_package_import(&record)
            .map_err(persistence_error)?
            .preview)
    }
}

pub(super) fn check_path(root: &Path, path: &Path) -> Result<(), ServiceError> {
    let relative = path.strip_prefix(root).map_err(|_| invalid())?;
    let mut current = root.to_path_buf();
    reject_symlink(&current)?;
    for part in relative.components() {
        if !matches!(part, std::path::Component::Normal(_)) {
            return Err(invalid());
        }
        current.push(part);
        reject_symlink(&current)?;
    }
    Ok(())
}

fn read_tree(
    base: &Path,
    path: &Path,
    files: &mut BTreeMap<String, Vec<u8>>,
) -> Result<(), ServiceError> {
    reject_symlink(path)?;
    for entry in fs::read_dir(path).map_err(|_| invalid())? {
        let entry = entry.map_err(|_| invalid())?;
        let kind = entry.file_type().map_err(|_| invalid())?;
        if kind.is_symlink() {
            return Err(invalid());
        }
        if kind.is_dir() {
            read_tree(base, &entry.path(), files)?;
            continue;
        }
        if !kind.is_file() || files.len() >= MAX_SOURCE_FILES + 3 {
            return Err(invalid());
        }
        let metadata = entry.metadata().map_err(|_| invalid())?;
        if metadata.len() > MAX_SOURCE_FILE_BYTES as u64 {
            return Err(invalid());
        }
        let path = entry.path();
        let relative = path
            .strip_prefix(base)
            .map_err(|_| invalid())?
            .to_str()
            .ok_or_else(invalid)?
            .replace(std::path::MAIN_SEPARATOR, "/");
        files.insert(relative, fs::read(path).map_err(|_| invalid())?);
    }
    Ok(())
}

pub(super) fn verify_package_tree(
    root: &Path,
    bytes: &[u8],
    published: bool,
) -> Result<(), ServiceError> {
    reject_symlink(root)?;
    let mut actual = BTreeMap::new();
    read_tree(root, &root.join("source"), &mut actual)?;
    read_tree(root, &root.join("generated"), &mut actual)?;
    if actual != decode_archive(bytes)? {
        return Err(ServiceError::new(
            "personal_package_import_candidate_changed",
        ));
    }
    let allowed = if published {
        vec![
            "source",
            "generated",
            "artifacts",
            ".cilai-package-import.json",
        ]
    } else {
        vec!["source", "generated", ".cilai-personal-install.json"]
    };
    for entry in fs::read_dir(root).map_err(|_| invalid())? {
        let entry = entry.map_err(|_| invalid())?;
        if !allowed.iter().any(|name| entry.file_name() == *name) {
            return Err(ServiceError::new(
                "personal_package_import_candidate_changed",
            ));
        }
    }
    Ok(())
}

fn import_marker(record: &PersonalPackageImportRecord) -> Value {
    json!({"schemaVersion":1,"profileId":record.preview.profile_id,"importId":record.preview.import_id,"previewDigest":record.preview.preview_digest,"localCreationProjectId":record.preview.local_creation_project_id,"localPackageDigest":record.preview.local_package_digest})
}

impl AppServerServices {
    fn publish_import_candidate(
        &self,
        record: &PersonalPackageImportRecord,
        version: &PersonalCapabilityVersion,
        bytes: &[u8],
    ) -> Result<(), ServiceError> {
        let p = &record.preview;
        let project_root = self
            .capability_projects_root()?
            .join(&p.local_creation_project_id);
        let destination = if matches!(p.target, PersonalPackageImportTarget::New {}) {
            project_root
        } else {
            project_root.join("imports").join(&p.import_id)
        };
        let profile = self.data_dir.as_deref().ok_or_else(invalid)?;
        check_path(profile, &destination)?;
        let marker = import_marker(record);
        let artifact_name = format!(
            "{}.zip",
            p.local_package_digest.trim_start_matches("sha256:")
        );
        if destination.exists() {
            let path = destination.join(".cilai-package-import.json");
            reject_symlink(&path)?;
            let existing: Value = serde_json::from_slice(&fs::read(path).map_err(|_| invalid())?)
                .map_err(|_| invalid())?;
            if existing != marker {
                return Err(ServiceError::new(
                    "personal_package_import_destination_conflict",
                ));
            }
            verify_package_tree(&destination, bytes, true)?;
            if read_archive(&destination.join("artifacts").join(&artifact_name))? != bytes {
                return Err(ServiceError::new(
                    "personal_package_import_candidate_changed",
                ));
            }
            return Ok(());
        }
        let parent = destination.parent().ok_or_else(invalid)?;
        fs::create_dir_all(parent)
            .map_err(|_| ServiceError::new("personal_package_import_write_failed"))?;
        check_path(profile, parent)?;
        let staging = parent.join(format!(".{}-{}", p.import_id, Uuid::new_v4().simple()));
        fs::create_dir(&staging)
            .map_err(|_| ServiceError::new("personal_package_import_write_failed"))?;
        for (path, content) in decode_archive(bytes)? {
            write_managed_file(&staging, &path, &content)?;
        }
        fs::create_dir(staging.join("artifacts")).map_err(|_| invalid())?;
        write_new_file(
            &staging.join("artifacts").join(artifact_name),
            bytes,
            "personal_package_import_write_failed",
        )?;
        write_new_file(
            &staging.join(".cilai-package-import.json"),
            &encode_json(&marker)?,
            "personal_package_import_write_failed",
        )?;
        sync_tree(&staging)?;
        if destination.exists() {
            return Err(ServiceError::new(
                "personal_package_import_destination_conflict",
            ));
        }
        fs::rename(&staging, &destination)
            .map_err(|_| ServiceError::new("personal_package_import_write_failed"))?;
        sync_ancestors(profile, parent)?;
        let expected = profile
            .join(&version.project.managed_relative_path)
            .join(&version.build.package_relative_path);
        if expected
            != destination.join("artifacts").join(format!(
                "{}.zip",
                p.local_package_digest.trim_start_matches("sha256:")
            ))
        {
            return Err(invalid());
        }
        Ok(())
    }

    pub(crate) fn personal_package_import_commit(
        &mut self,
        p: &PersonalPackageImportCommitParams,
        now: i64,
    ) -> Result<PersonalPackageImportCommitResult, ServiceError> {
        validate_text(&p.client_operation_id, 160, "client_operation_id_invalid")?;
        let record = self.import_record(&p.import_id)?;
        if record.preview.preview_digest != p.preview_digest {
            return Err(ServiceError::new("personal_package_import_preview_changed"));
        }
        let permissions = normalized_permissions(p, &record.preview)?;
        let request = fingerprint(
            &json!({"importId":p.import_id,"previewDigest":p.preview_digest,"confirmedPermissions":permissions,"clientOperationId":p.client_operation_id}),
        )?;
        match record.preview.state {
            PersonalPackageImportState::Cancelled => {
                return Err(ServiceError::new("personal_package_import_cancelled"));
            }
            PersonalPackageImportState::Committed => {
                if record.commit_operation_id.as_deref() != Some(p.client_operation_id.as_str())
                    || record.commit_fingerprint.as_deref() != Some(request.as_str())
                {
                    return Err(ServiceError::new(
                        "personal_package_import_already_committed",
                    ));
                }
                return import_replay(p, &record);
            }
            PersonalPackageImportState::Prepared => (),
        }
        let (version, bytes) = self.verified_import_candidate(&record, now)?;
        self.publish_import_candidate(&record, &version, &bytes)?;
        let install_params = CapabilityCreationInstallParams {
            creation_project_id: record.preview.local_creation_project_id.clone(),
            package_digest: record.preview.local_package_digest.clone(),
            expected_revision: record.preview.expected_revision.unwrap_or(1),
            client_operation_id: p.client_operation_id.clone(),
            request_fingerprint: request.clone(),
        };
        let installed = materialize_personal_web_install(
            &self.personal_capabilities_root()?,
            &install_params,
            &version.project,
            &bytes,
            now,
        )?;
        verify_package_tree(&installed.final_directory, &bytes, false)?;
        sync_tree(&installed.final_directory)?;
        sync_ancestors(
            self.data_dir.as_deref().ok_or_else(invalid)?,
            installed.final_directory.parent().ok_or_else(invalid)?,
        )?;
        let data_params = p
            .data_change
            .as_ref()
            .ok_or_else(|| ServiceError::new("personal_application_data_operation_required"))?;
        let data = self.prepare_application_data_copy(
            data_params,
            &super::data::Activation {
                client: &p.client_operation_id,
                project: &record.preview.local_creation_project_id,
                digest: &record.preview.local_package_digest,
                revision: record.preview.expected_revision.unwrap_or(1),
                action: contexthub_protocol::PersonalApplicationDataAction::ImportUpgrade,
            },
            now,
        )?;
        let mut result = PersonalPackageImportCommitResult {
            import_id: p.import_id.clone(),
            local_package_digest: record.preview.local_package_digest.clone(),
            project: project_from_record(version.project.clone())?,
            replayed: false,
        };
        let value = serde_json::to_value(&result).map_err(|_| invalid())?;
        let committed =
            self.persistence_mut()?
                .commit_personal_package_import(&PersonalPackageImportCommand {
                    profile_id: &record.preview.profile_id,
                    data_operation_id: &data.operation.operation_id,
                    import_id: &p.import_id,
                    preview_digest: &p.preview_digest,
                    client_operation_id: &p.client_operation_id,
                    fingerprint: &request,
                    version: &version,
                    installed_relative_path: &installed.relative_location,
                    result: &value,
                    occurred_at: now,
                });
        result.replayed = match committed {
            Ok(replayed) => replayed,
            Err(failure) => {
                self.fail_application_data(
                    &data.operation.operation_id,
                    "personal_application_data_activation_unconfirmed",
                    now,
                )?;
                return Err(persistence_error(failure));
            }
        };
        Ok(result)
    }
}

fn import_replay(
    p: &PersonalPackageImportCommitParams,
    record: &PersonalPackageImportRecord,
) -> Result<PersonalPackageImportCommitResult, ServiceError> {
    let mut result: PersonalPackageImportCommitResult =
        serde_json::from_value(record.result.clone().ok_or_else(invalid)?)
            .map_err(|_| invalid())?;
    if result.import_id != p.import_id
        || result.project.creation_project_id != record.preview.local_creation_project_id
        || result.local_package_digest != record.preview.local_package_digest
    {
        return Err(invalid());
    }
    result.replayed = true;
    Ok(result)
}
fn normalized_permissions(
    p: &PersonalPackageImportCommitParams,
    preview: &PersonalPackageImportPreview,
) -> Result<Vec<String>, ServiceError> {
    let mut permissions = p.confirmed_permissions.clone();
    permissions.sort();
    if permissions.windows(2).any(|w| w[0] == w[1]) || permissions != preview.requested_permissions
    {
        return Err(ServiceError::new(
            "personal_package_import_permissions_unconfirmed",
        ));
    }
    Ok(permissions)
}

fn build_record(
    p: &PersonalPackageImportPreview,
    created_at: i64,
    acceptance_examples_json: String,
) -> Result<PersonalCapabilityVersion, ServiceError> {
    let name = format!(
        "{}.zip",
        p.local_package_digest
            .strip_prefix("sha256:")
            .ok_or_else(invalid)?
    );
    let relative = if matches!(p.target, PersonalPackageImportTarget::New {}) {
        format!("artifacts/{name}")
    } else {
        format!("imports/{}/artifacts/{name}", p.import_id)
    };
    Ok(PersonalCapabilityVersion {
        version: p.local_version.clone(),
        project: CapabilityCreationProjectRecord {
            creation_project_id: p.local_creation_project_id.clone(),
            title: p.source.title.clone(),
            business_goal: p.source.summary.clone(),
            target_type: p.source.target_type.clone(),
            status: "installed".into(),
            conversation_id: None,
            managed_relative_path: format!("capability-projects/{}", p.local_creation_project_id),
            requested_permissions_json: serde_json::to_string(&p.requested_permissions)
                .map_err(|_| invalid())?,
            acceptance_examples_json,
            source_digest: Some(p.source.source_digest.clone()),
            source_file_count: i64::from(p.source.source_file_count),
            state_revision: p
                .expected_revision
                .unwrap_or(1)
                .checked_add(1)
                .ok_or_else(invalid)?,
            package_digest: Some(p.local_package_digest.clone()),
            error_reason: None,
            created_at,
            updated_at: p.updated_at,
        },
        build: CapabilityCreationBuildRecord {
            build_id: format!("build_{}", p.import_id.trim_start_matches("import_")),
            creation_project_id: p.local_creation_project_id.clone(),
            source_digest: p.source.source_digest.clone(),
            package_digest: p.local_package_digest.clone(),
            package_relative_path: relative,
            file_count: i64::from(p.source.source_file_count),
            total_bytes: 0,
            created_at: p.updated_at,
        },
    })
}

impl AppServerServices {
    fn verified_import_candidate(
        &self,
        record: &PersonalPackageImportRecord,
        now: i64,
    ) -> Result<(PersonalCapabilityVersion, Vec<u8>), ServiceError> {
        let p = &record.preview;
        let root = self.package_import_root(&p.import_id)?;
        let profile = self.data_dir.as_deref().ok_or_else(invalid)?;
        check_path(profile, &root.join("archive.zip"))?;
        let source = inspect(&read_archive(&root.join("archive.zip"))?)?;
        if source.source != p.source || source.permissions != p.requested_permissions {
            return Err(ServiceError::new(
                "personal_package_import_candidate_changed",
            ));
        }
        let old = if let PersonalPackageImportTarget::Upgrade { .. } = p.target {
            let state = self.import_target(&p.target, &p.source.target_type)?;
            if state.installed_digest != p.installed_digest || state.version != p.local_version {
                return Err(ServiceError::new("personal_package_import_target_changed"));
            }
            let old = self
                .persistence()?
                .capability_creation_project(&p.local_creation_project_id)
                .map_err(persistence_error)?
                .ok_or_else(invalid)?;
            Some(old)
        } else {
            None
        };
        let mut version = build_record(p, now, "[]".into())?;
        if let Some(old) = old {
            version.project.created_at = old.created_at;
            version.project.business_goal = old.business_goal;
            version.project.acceptance_examples_json = old.acceptance_examples_json;
            version.project.conversation_id = old.conversation_id;
        }
        version.project.updated_at = now;
        version.build.created_at = now;
        let path = root.join("candidate/artifacts").join(format!(
            "{}.zip",
            p.local_package_digest.trim_start_matches("sha256:")
        ));
        check_path(profile, &path)?;
        let bytes = read_archive(&path)?;
        if hash(&bytes) != p.local_package_digest {
            return Err(ServiceError::new(
                "personal_package_import_candidate_changed",
            ));
        }
        validate_personal_export(&bytes, &version.project, &p.local_version)?;
        version.build.total_bytes = i64::try_from(bytes.len()).map_err(|_| invalid())?;
        Ok((version, bytes))
    }
}

use super::{
    AppServerServices, Digest, Path, PersonalApplicationOpenParams, ServiceError, Sha256, Uuid, fs,
    json, persistence_error, reject_symlink, write_new_file,
};
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::Read as _;

const MAX_FILE_BYTES: u64 = 128 * 1024;
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PersonalFileSelectParams {
    creation_project_id: String,
    package_digest: String,
    path: String,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PersonalFileAccessParams {
    creation_project_id: String,
    package_digest: String,
    grant_id: String,
    #[serde(default)]
    content: Option<String>,
}

fn read_selected(path: &Path) -> Result<Vec<u8>, ServiceError> {
    reject_symlink(path)?;
    let file = fs::File::open(path).map_err(|_| ServiceError::new("personal_file_unavailable"))?;
    if !file
        .metadata()
        .is_ok_and(|metadata| metadata.is_file() && metadata.len() <= MAX_FILE_BYTES)
    {
        return Err(ServiceError::new("personal_file_size_invalid"));
    }
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ServiceError::new("personal_file_unavailable"))?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(ServiceError::new("personal_file_size_invalid"));
    }
    Ok(bytes)
}

impl AppServerServices {
    fn personal_file_permissions(
        &mut self,
        id: &str,
        digest: &str,
        now: i64,
    ) -> Result<Vec<String>, ServiceError> {
        let binding = self.personal_capability_runtime(
            &PersonalApplicationOpenParams {
                creation_project_id: id.into(),
            },
            now,
        )?;
        if binding["packageDigest"] != digest {
            return Err(ServiceError::new("personal_capability_state_conflict"));
        }
        let project = self
            .persistence()?
            .capability_creation_project(id)
            .map_err(persistence_error)?
            .ok_or_else(|| ServiceError::new("capability_creation_project_not_found"))?;
        serde_json::from_str(&project.requested_permissions_json)
            .map_err(|_| ServiceError::new("capability_creation_permissions_invalid"))
    }

    pub(crate) fn personal_file_select(
        &mut self,
        params: &PersonalFileSelectParams,
        now: i64,
    ) -> Result<Value, ServiceError> {
        let permissions = self.personal_file_permissions(
            &params.creation_project_id,
            &params.package_digest,
            now,
        )?;
        if !permissions
            .iter()
            .any(|p| p == "personal_file.read" || p == "personal_file.write_confirmed")
        {
            return Err(ServiceError::new("personal_file_permission_required"));
        }
        let path = Path::new(&params.path);
        if !path.is_absolute() {
            return Err(ServiceError::new("personal_file_path_invalid"));
        }
        let bytes = read_selected(path)?;
        let path = path
            .canonicalize()
            .map_err(|_| ServiceError::new("personal_file_unavailable"))?;
        let id = format!("file_{}", Uuid::new_v4().simple());
        let name = path
            .file_name()
            .and_then(|v| v.to_str())
            .ok_or_else(|| ServiceError::new("personal_file_path_invalid"))?;
        let digest = format!("sha256:{:x}", Sha256::digest(&bytes));
        self.persistence()?.save_personal_file_grant(&json!({"grantId":id,"creationProjectId":params.creation_project_id,
            "packageDigest":params.package_digest,"path":path,"name":name,"digest":digest,"expiresAt":now+10*60*1000,
            "canWrite":permissions.iter().any(|p| p=="personal_file.write_confirmed")})).map_err(persistence_error)?;
        Ok(
            json!({"grantId":id,"name":name,"size":bytes.len(),"packageDigest":params.package_digest}),
        )
    }

    pub(crate) fn personal_file_consume(
        &mut self,
        params: &PersonalFileAccessParams,
        now: i64,
    ) -> Result<Value, ServiceError> {
        self.personal_file_permissions(&params.creation_project_id, &params.package_digest, now)?;
        let grant = self
            .persistence_mut()?
            .personal_file_grant(
                &params.grant_id,
                &params.creation_project_id,
                &params.package_digest,
                now,
                true,
            )
            .map_err(persistence_error)?;
        let bytes = read_selected(Path::new(
            grant["path"]
                .as_str()
                .ok_or_else(|| ServiceError::new("personal_file_path_invalid"))?,
        ))?;
        if grant["digest"] != format!("sha256:{:x}", Sha256::digest(&bytes)) {
            return Err(ServiceError::new("personal_file_changed"));
        }
        Ok(
            json!({"grantId":params.grant_id,"name":grant["name"],"encoding":"base64","content":base64::engine::general_purpose::STANDARD.encode(bytes),"canWrite":grant["canWrite"]}),
        )
    }

    fn personal_file_write_grant(
        &mut self,
        params: &PersonalFileAccessParams,
        now: i64,
    ) -> Result<Value, ServiceError> {
        let permissions = self.personal_file_permissions(
            &params.creation_project_id,
            &params.package_digest,
            now,
        )?;
        if !permissions
            .iter()
            .any(|p| p == "personal_file.write_confirmed")
        {
            return Err(ServiceError::new("personal_file_permission_required"));
        }
        let grant = self
            .persistence_mut()?
            .personal_file_grant(
                &params.grant_id,
                &params.creation_project_id,
                &params.package_digest,
                now,
                false,
            )
            .map_err(persistence_error)?;
        let _content = params
            .content
            .as_ref()
            .filter(|c| c.len() as u64 <= MAX_FILE_BYTES)
            .ok_or_else(|| ServiceError::new("personal_file_size_invalid"))?;
        let path = Path::new(
            grant["path"]
                .as_str()
                .ok_or_else(|| ServiceError::new("personal_file_path_invalid"))?,
        );
        if grant["canWrite"] != true
            || grant["digest"] != format!("sha256:{:x}", Sha256::digest(read_selected(path)?))
        {
            return Err(ServiceError::new("personal_file_changed"));
        }
        if path
            .canonicalize()
            .map_err(|_| ServiceError::new("personal_file_unavailable"))?
            != path
        {
            return Err(ServiceError::new("personal_file_changed"));
        }
        Ok(grant)
    }

    pub(crate) fn personal_file_write_preview(
        &mut self,
        params: &PersonalFileAccessParams,
        now: i64,
    ) -> Result<Value, ServiceError> {
        let grant = self.personal_file_write_grant(params, now)?;
        Ok(json!({"name":grant["name"],"path":grant["path"]}))
    }

    pub(crate) fn personal_file_write(
        &mut self,
        params: &PersonalFileAccessParams,
        now: i64,
    ) -> Result<Value, ServiceError> {
        let grant = self.personal_file_write_grant(params, now)?;
        let content = params
            .content
            .as_ref()
            .ok_or_else(|| ServiceError::new("personal_file_size_invalid"))?;
        let path = Path::new(
            grant["path"]
                .as_str()
                .ok_or_else(|| ServiceError::new("personal_file_path_invalid"))?,
        );
        let pending = path.with_file_name(format!(".cilai-{}.tmp", Uuid::new_v4().simple()));
        write_new_file(&pending, content.as_bytes(), "personal_file_write_failed")?;
        let result = (|| {
            if grant["digest"] != format!("sha256:{:x}", Sha256::digest(read_selected(path)?)) {
                return Err(ServiceError::new("personal_file_changed"));
            }
            fs::rename(&pending, path)
                .map_err(|_| ServiceError::new("personal_file_write_failed"))?;
            let digest = format!("sha256:{:x}", Sha256::digest(content.as_bytes()));
            self.persistence()?
                .finish_personal_file_write(&params.grant_id, &digest)
                .map_err(persistence_error)?;
            Ok(json!({"name":grant["name"],"digest":digest,"written":true}))
        })();
        if pending.exists() {
            let _ = fs::remove_file(pending);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use super::*;
    #[test]
    fn selected_file_is_bound_to_one_use_current_bytes_and_write_permission() {
        let root = super::super::data::test_support::profile_root("cilai-file-broker");
        fs::create_dir_all(&root).unwrap();
        let persistence =
            super::super::data::test_support::ready_db(&root, "test.db", "legacy-file-test");
        let mut service = AppServerServices::with_data_dir(Some(persistence), Some(root.clone()));
        let mut create = CapabilityCreationCreateParams {
            title: "文件检查".into(),
            business_goal: "检查选定文件".into(),
            target_type: "local_mcp".into(),
            conversation_id: None,
            requested_permissions: vec![
                "personal_file.read".into(),
                "personal_file.write_confirmed".into(),
            ],
            acceptance_examples: vec![CapabilityCreationAcceptanceExample {
                input: "选择文件".into(),
                expected: "返回检查结果".into(),
            }],
            source_files: vec![CapabilityCreationSourceFile {
                path: "mcp/server.mjs".into(),
                content: "console.log('ready')".into(),
            }],
            client_operation_id: "file-project".into(),
            request_fingerprint: String::new(),
        };
        create.request_fingerprint = capability_creation_request_fingerprint(&create);
        let project = service
            .capability_creation_create(&create, 1)
            .unwrap()
            .project;
        let mut install = CapabilityCreationInstallParams {
            creation_project_id: project.creation_project_id.clone(),
            expected_revision: project.state_revision,
            package_digest: project.package_digest.unwrap(),
            client_operation_id: "install-file".into(),
            request_fingerprint: String::new(),
        };
        install.request_fingerprint = capability_install_request_fingerprint(&install);
        service.capability_creation_install(&install, 2).unwrap();
        let file = root.join("selected.txt");
        fs::write(&file, "员工文件").unwrap();
        let select = PersonalFileSelectParams {
            creation_project_id: install.creation_project_id.clone(),
            package_digest: install.package_digest.clone(),
            path: file.to_string_lossy().into(),
        };
        let grant = service.personal_file_select(&select, 10).unwrap();
        assert!(grant.get("path").is_none());
        let mut access = PersonalFileAccessParams {
            creation_project_id: select.creation_project_id.clone(),
            package_digest: select.package_digest.clone(),
            grant_id: grant["grantId"].as_str().unwrap().into(),
            content: Some("整理后".into()),
        };
        assert!(service.personal_file_write_preview(&access, 11).is_err());
        assert!(service.personal_file_write(&access, 11).is_err());
        let read = service.personal_file_consume(&access, 12).unwrap();
        assert_eq!(
            read["content"],
            base64::engine::general_purpose::STANDARD.encode("员工文件")
        );
        assert!(read.get("path").is_none());
        assert!(service.personal_file_consume(&access, 13).is_err());
        let preview = service.personal_file_write_preview(&access, 13).unwrap();
        assert_eq!(preview["name"], "selected.txt");
        assert_eq!(
            preview["path"],
            file.canonicalize().unwrap().to_string_lossy().as_ref()
        );
        assert_eq!(
            service.personal_file_write(&access, 14).unwrap()["written"],
            true
        );
        assert_eq!(fs::read_to_string(&file).unwrap(), "整理后");
        assert!(service.personal_file_write(&access, 15).is_err());
        access.grant_id = service.personal_file_select(&select, 20).unwrap()["grantId"]
            .as_str()
            .unwrap()
            .into();
        fs::write(&file, "用户在外部修改").unwrap();
        assert!(service.personal_file_consume(&access, 21).is_err());
        access.grant_id = service.personal_file_select(&select, 30).unwrap()["grantId"]
            .as_str()
            .unwrap()
            .into();
        assert!(
            service
                .personal_file_consume(&access, 30 + 10 * 60 * 1000)
                .is_err()
        );
        access.grant_id = service.personal_file_select(&select, 40).unwrap()["grantId"]
            .as_str()
            .unwrap()
            .into();
        access.package_digest = format!("sha256:{}", "0".repeat(64));
        assert!(service.personal_file_consume(&access, 41).is_err());
        drop(service);
        fs::remove_dir_all(root).unwrap();
    }
}

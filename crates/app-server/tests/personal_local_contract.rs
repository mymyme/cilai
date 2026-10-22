use contexthub_application::{UnitOfWork, WorkspaceRecord, WorkspaceRepository};
use contexthub_domain::{LocalWorkspaceStatus, WorkspaceId, WorkspaceKind};
use contexthub_persistence_sqlite::SqlitePersistence;
use serde_json::{Value, json};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

struct Server {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
}

impl Server {
    fn start(profile: &Path, origin: &str) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_contexthub-app-server"));
        command.env_clear();
        #[cfg(windows)]
        if let Some(system_root) = std::env::var_os("SystemRoot") {
            command.env("SystemRoot", system_root);
        }
        let mut child = command
            .env("HOME", profile.join("home"))
            .env("CILAI_LSP_ORIGIN", origin)
            .env("CILAI_COLLABORATION_ORIGIN", origin)
            .args([
                "serve",
                "--transport",
                "stdio",
                "--enable-local-ipc",
                "--environment",
                "personal",
                "--data-dir",
            ])
            .arg(profile)
            .arg("--runtime-dir")
            .arg(profile.join("run"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let output = BufReader::new(child.stdout.take().unwrap());
        let mut server = Self {
            child,
            input,
            output,
        };
        let initialized = server.request("initialize", json!({"client":{"name":"contexthub-desktop","version":"0.1.0","platform":std::env::consts::OS,"arch":std::env::consts::ARCH},"protocol":{"minVersion":"1.0","maxVersion":"1.0"}}));
        assert!(initialized.get("result").is_some(), "{initialized}");
        writeln!(
            server.input,
            "{}",
            json!({"jsonrpc":"2.0","method":"initialized","params":{}})
        )
        .unwrap();
        server.input.flush().unwrap();
        server
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let mut request = json!({"jsonrpc":"2.0","id":method,"method":method});
        request["params"] = params;
        writeln!(self.input, "{request}").unwrap();
        self.input.flush().unwrap();
        let mut line = String::new();
        if self.output.read_line(&mut line).unwrap() == 0 {
            let mut stderr = String::new();
            self.child
                .stderr
                .take()
                .unwrap()
                .read_to_string(&mut stderr)
                .unwrap();
            panic!("closed before {method}: {stderr}");
        }
        let value: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(value["id"], method);
        value
    }

    fn stop(mut self) {
        assert_eq!(
            self.request("shutdown/prepare", json!({}))["result"]["ready"],
            true
        );
        assert_eq!(
            self.request("shutdown/execute", json!({}))["result"]["accepted"],
            true
        );
        assert!(self.child.wait().unwrap().success());
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        if self.child.try_wait().unwrap().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

const REMOVED_METHODS: &[&str] = &[
    "capture/execute",
    "capture/ingest",
    "cloudWorkspace/createTeamSpace",
    "cloudWorkspace/attach",
    "cloudWorkspace/fileAction",
    "cloudWorkspace/material",
    "cloudWorkspace/preview",
    "cloudWorkspace/deleteItem",
    "cloudWorkspace/start",
    "cloudWorkspace/status",
    "cloudWorkspace/teamSpaces",
    "syncPlan/create",
    "syncPlan/get",
    "sync/execute",
    "syncExecution/get",
    "conflict/list",
    "conflict/resolve",
    "applicationCatalog/list",
    "applicationCatalog/launch",
    "aiBrowserPolicy/get",
    "employeeCredential/get",
    "employeeCredential/save",
    "employeeCredential/delete",
];

const LEGACY_SOURCE_METHODS: &[&str] = &[
    "workspace/scan",
    "workspace/files",
    "workspace/listDirectory",
    "workspace/targetValidate",
    "workspace/createDirectory",
    "preview/open",
    "preview/readBinary",
    "file/hydrate",
    "file/confirm",
    "textDiff/compute",
    "project/attachWorkspace",
];

fn create_local_project_and_confirm(
    server: &mut Server,
    directory: &Path,
) -> (String, String, String) {
    let created = server.request("project/create", json!({"displayName":"无模型项目","goal":"本地验证","workspaceIds":[],"teamSpaceIds":[],"clientOperationId":"project-create","requestFingerprint":"project-create"}));
    let project_id = created["result"]["project"]["projectId"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(created["result"]["project"]["primaryWorkspaceId"].is_null());
    let search = server.request(
        "unifiedSearch/query",
        json!({"query":"无模型","scope":["local"],"limit":8,"cursors":[]}),
    );
    assert!(
        search["result"]["groups"][0]["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["objectId"] == project_id)
    );
    let connected = server.request("workspace/create", json!({"localRoot":directory,"displayName":"本地目录","clientOperationId":"workspace-create"}));
    let workspace_id = connected["result"]["workspace"]["workspaceId"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(server.request("project/attachWorkspace", json!({"projectId":project_id,"workspaceIds":[workspace_id],"clientOperationId":"attach","requestFingerprint":"attach"})).get("error").is_none());
    assert!(
        server
            .request("workspace/scan", json!({"workspaceId":workspace_id}))
            .get("error")
            .is_none()
    );
    let files = server.request(
        "workspace/files",
        json!({"workspaceId":workspace_id,"limit":20}),
    );
    let file_id = files["result"]["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|file| file["relativePath"] == "note.md")
        .unwrap()["localFileId"]
        .as_str()
        .unwrap()
        .to_owned();
    let preview = server.request(
        "preview/open",
        json!({"workspaceId":workspace_id,"localFileId":file_id,"source":"local"}),
    );
    assert_eq!(preview["result"]["availability"], "ready");
    let revision = preview["result"]["contentRevision"].as_str().unwrap();
    let mut confirm = json!({"workspaceId":workspace_id,"localFileId":file_id,"expectedChecksumAlgorithm":"sha256","expectedChecksumValue":revision,"expectedBaselineVersionId":null,"clientOperationId":"confirm"});
    assert_eq!(
        server.request("file/confirm", confirm.clone())["result"]["replayed"],
        false
    );
    assert_eq!(
        server.request(
            "file/confirmationHistory",
            json!({"workspaceId":workspace_id,"localFileId":file_id})
        )["result"]["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    fs::write(directory.join("note.md"), "外部修改").unwrap();
    confirm["clientOperationId"] = json!("confirm-after-change");
    assert!(
        server
            .request("file/confirm", confirm)
            .get("error")
            .is_some()
    );
    (project_id, workspace_id, file_id)
}

fn create_personal_profile() -> (PathBuf, PathBuf) {
    let root = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("cilai-personal-contract-{}", uuid::Uuid::new_v4()));
    let profile_id = uuid::Uuid::new_v4().simple().to_string();
    let business = root.join("cn.cilai.personal.workbench/business");
    let profile = business.join("profiles").join(&profile_id);
    fs::create_dir_all(&profile).unwrap();
    let mut manifest = contexthub_platform_adapter::security::create_new_restricted_file(
        &business.join("local-profile.json"),
    )
    .unwrap();
    write!(
        manifest,
        "{}",
        json!({"schemaVersion":1,"profileId":profile_id,"displayName":"本地资料"})
    )
    .unwrap();
    (root, profile)
}

#[test]
fn local_project_files_confirmation_and_history_work_without_enterprise_services() {
    let (root, profile) = create_personal_profile();
    let directory = root.join("workspace");
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("note.md"), "本地资料").unwrap();
    let sentinel = TcpListener::bind("127.0.0.1:0").unwrap();
    sentinel.set_nonblocking(true).unwrap();
    let origin = format!("http://{}", sentinel.local_addr().unwrap());
    let mut server = Server::start(&profile, &origin);
    let (project_id, workspace_id, file_id) =
        create_local_project_and_confirm(&mut server, &directory);
    for &method in REMOVED_METHODS {
        assert_eq!(
            server.request(method, json!({}))["error"]["data"]["reason"],
            "personal_feature_removed",
            "{method}"
        );
    }
    assert_eq!(
        server.request(
            "unifiedSearch/query",
            json!({"query":"本地","scope":["knowledge"],"limit":8,"cursors":[]})
        )["error"]["data"]["reason"],
        "personal_search_scope_not_supported"
    );
    server.stop();
    let mut db = SqlitePersistence::open(&profile.join("contexthub.db"), "0.1.0").unwrap();
    db.transact(|repositories| {
        repositories.insert_workspace(&WorkspaceRecord {
            workspace_id: WorkspaceId::new("historical-cloud"),
            workspace_kind: WorkspaceKind::CloudTeam,
            display_name: "旧云来源".into(),
            local_root: root
                .join("retired-cloud-source")
                .to_string_lossy()
                .into_owned(),
            local_root_identity: None,
            local_status: LocalWorkspaceStatus::Ready,
            state_revision: 0,
            last_local_scan_at: None,
            created_at: 1,
            updated_at: 1,
        })
    })
    .unwrap();
    drop(db);
    let mut server = Server::start(&profile, &origin);
    assert_eq!(
        server.request("project/get", json!({"projectId":project_id}))["result"]["project"]["projectId"],
        project_id
    );
    assert_eq!(
        server.request(
            "file/confirmationHistory",
            json!({"workspaceId":workspace_id,"localFileId":file_id})
        )["result"]["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    for &method in LEGACY_SOURCE_METHODS {
        assert_eq!(
            server.request(method, json!({"workspaceId":"historical-cloud"}))["error"]["data"]["reason"],
            "personal_cloud_source_unavailable",
            "{method}"
        );
    }
    let listing = server.request("workspace/list", json!({}));
    assert!(
        listing["result"]["workspaces"]
            .as_array()
            .unwrap()
            .iter()
            .any(|workspace| workspace["workspaceId"] == "historical-cloud"
                && workspace["workspaceKind"] == "cloud_team")
    );
    server.stop();
    assert!(
        matches!(sentinel.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
    );
    assert_eq!(
        fs::read_to_string(directory.join("note.md")).unwrap(),
        "外部修改"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn credential_cleanup_receipts_remain_scoped_across_real_stdio_restarts() {
    let (root, profile) = create_personal_profile();
    let sentinel = TcpListener::bind("127.0.0.1:0").unwrap();
    sentinel.set_nonblocking(true).unwrap();
    let origin = format!("http://{}", sentinel.local_addr().unwrap());
    let mut server = Server::start(&profile, &origin);
    let prepared = server.request("modelConnection/savePrepare",json!({"operationId":"metadata-only-cancelled-key","config":{"name":"凭据元数据测试","kind":"openai_key","authMode":"api_key","enabled":true},"credentialAction":"replace"}));
    assert_eq!(prepared["result"]["state"], "prepared");
    server.request(
        "modelConnection/saveAbort",
        json!({"operationId":"metadata-only-cancelled-key"}),
    );
    let listing = server.request("modelCredentialCleanup/list", json!({}));
    let candidate = &listing["result"][0];
    assert_eq!(candidate["canPrepare"], true);
    let response = server.request("modelCredentialCleanup/prepare",json!({"cleanupId":candidate["cleanupId"],"expectedReferenceDigest":candidate["referenceDigest"]}));
    let plan = response["result"].clone();
    assert_eq!(plan["state"], "pending");
    server.stop();
    let mut server = Server::start(&profile, &origin);
    assert_eq!(
        server.request("modelCredentialCleanup/list", json!({}))["result"][0]["plan"],
        plan
    );
    assert_eq!(server.request("modelCredentialCleanup/ack",json!({"cleanupId":plan["cleanupId"],"expectedReferenceDigest":plan["referenceDigest"],"credentialRef":{"service":"Codex Auth","account":"unrelated"}}))["error"]["data"]["reason"],"model_credential_cleanup_receipt_mismatch");
    let confirmed = server.request("modelCredentialCleanup/ack",json!({"cleanupId":plan["cleanupId"],"expectedReferenceDigest":plan["referenceDigest"],"credentialRef":plan["credentialRef"]}));
    assert_eq!(confirmed["result"]["state"], "confirmed");
    assert_eq!(
        server.request(
            "modelConnection/changeStatus",
            json!({"operationId":"metadata-only-cancelled-key"})
        )["result"]["credentialCleanup"],
        "confirmed"
    );
    assert_eq!(
        server.request("modelCredentialCleanup/unknown", json!({}))["error"]["data"]["reason"],
        "model_method_not_found"
    );
    server.stop();
    assert!(
        matches!(sentinel.accept(),Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
    );
    fs::remove_dir_all(root).unwrap();
}

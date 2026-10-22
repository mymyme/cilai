use super::{Admission, NativeProxy};
use crate::model_connections::{ResolvedNetwork, system_proxy};
use base64::Engine as _;
use contexthub_protocol::{
    MODEL_NETWORK_CREDENTIAL_SERVICE, ModelNetworkAuthentication, ModelNetworkConfig,
    ModelNetworkMode, ModelSecretRef,
};
use std::io::{Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::time::Duration;

struct Credential {
    reference: ModelSecretRef,
    password: String,
    vault: contexthub_platform_adapter::credential_vault::LocalCredentialVault,
    root: std::path::PathBuf,
}
impl Credential {
    fn new() -> Self {
        let reference = ModelSecretRef {
            service: MODEL_NETWORK_CREDENTIAL_SERVICE.into(),
            account: format!(
                "{}:{}:1",
                uuid::Uuid::new_v4().simple(),
                uuid::Uuid::new_v4().simple()
            ),
        };
        let password = format!("synthetic-{}", uuid::Uuid::new_v4().simple());
        let root = crate::ai_runtime::model_test_fixture::fixture_root("native-vault");
        contexthub_platform_adapter::security::ensure_restricted_directory(&root).unwrap();
        let vault = contexthub_platform_adapter::credential_vault::LocalCredentialVault::new(
            root.join("credentials"),
        )
        .unwrap();
        let credential = Self {
            reference,
            password,
            vault,
            root,
        };
        let mut bytes = serde_json::to_vec(&serde_json::json!({"proxyBasic":{"username":"synthetic-user","password":credential.password},"serviceBasic":null,"headers":{}})).unwrap();
        let result = credential.vault.write(
            &credential.reference.service,
            &credential.reference.account,
            &bytes,
        );
        bytes.fill(0);
        result.unwrap();
        credential
    }
    fn route(&self, port: u16) -> ResolvedNetwork {
        system_proxy::resolve(
            &ModelNetworkConfig {
                mode: ModelNetworkMode::CustomProxy,
                proxy_url: Some(format!("http://127.0.0.1:{port}")),
                proxy_secret_ref: Some(self.reference.clone()),
                proxy_auth: ModelNetworkAuthentication::Basic,
                ..Default::default()
            },
            &self.root.join("credentials"),
        )
        .unwrap()
    }
    fn confirm_cleanup(&self) {
        self.vault
            .delete(&self.reference.service, &self.reference.account)
            .unwrap();
        assert!(
            self.vault
                .read(&self.reference.service, &self.reference.account)
                .unwrap()
                .is_none()
        );
    }
}
impl Drop for Credential {
    fn drop(&mut self) {
        let _ = self
            .vault
            .delete(&self.reference.service, &self.reference.account);
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}

fn headers(socket: &mut TcpStream) -> String {
    let mut bytes = Vec::new();
    while !bytes.ends_with(b"\r\n\r\n") && bytes.len() < 32768 {
        let mut byte = [0_u8];
        socket.read_exact(&mut byte).unwrap();
        bytes.push(byte[0]);
    }
    String::from_utf8(bytes).unwrap()
}

fn connect(bridge: &NativeProxy, target: &str, authenticated: bool) -> TcpStream {
    let url = reqwest::Url::parse(&bridge.endpoint).unwrap();
    let mut socket = TcpStream::connect((url.host_str().unwrap(), url.port().unwrap())).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let authorization = if authenticated {
        format!(
            "Proxy-Authorization: Basic {}\r\n",
            base64::engine::general_purpose::STANDARD.encode(format!(
                "{}:{}",
                url.username(),
                url.password().unwrap()
            ))
        )
    } else {
        String::new()
    };
    write!(
        socket,
        "CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n{authorization}\r\n"
    )
    .unwrap();
    socket
}

fn admission(allowed: Arc<AtomicBool>) -> Admission {
    Arc::new(move || {
        if allowed.load(Ordering::Acquire) {
            Ok(Box::new(()) as Box<dyn Send>)
        } else {
            Err("synthetic_scope_revoked".into())
        }
    })
}

#[test]
fn native_proxy_requires_nonce_and_fixed_destination_before_any_upstream_connection() {
    let credential = Credential::new();
    let upstream = TcpListener::bind("127.0.0.1:0").unwrap();
    upstream.set_nonblocking(true).unwrap();
    let route = credential.route(upstream.local_addr().unwrap().port());
    let bridge = NativeProxy::start(
        route,
        vec!["fixture.invalid:443".into()],
        admission(Arc::new(AtomicBool::new(true))),
    )
    .unwrap();
    for (target, authenticated) in [("fixture.invalid:443", false), ("other.invalid:443", true)] {
        let mut socket = connect(&bridge, target, authenticated);
        assert!(headers(&mut socket).starts_with("HTTP/1.1 407 "));
    }
    assert_eq!(
        upstream.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    bridge.shutdown_checked().unwrap();
    bridge.shutdown_checked().unwrap();
    credential.confirm_cleanup();
}

#[test]
fn native_proxy_injects_only_local_proxy_credentials_relays_opaque_bytes_and_closes_on_revocation()
{
    let credential = Credential::new();
    let upstream = TcpListener::bind("127.0.0.1:0").unwrap();
    let route = credential.route(upstream.local_addr().unwrap().port());
    let allowed = Arc::new(AtomicBool::new(true));
    let bridge = NativeProxy::start(
        route.clone(),
        vec!["fixture.invalid:443".into()],
        admission(Arc::clone(&allowed)),
    )
    .unwrap();
    let expected = format!(
        "Proxy-Authorization: Basic {}",
        base64::engine::general_purpose::STANDARD
            .encode(format!("synthetic-user:{}", credential.password))
    );
    let (done_sender, done) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let (mut socket, _) = upstream.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let raw = headers(&mut socket);
        assert!(raw.starts_with("CONNECT fixture.invalid:443 HTTP/1.1\r\n"));
        assert!(raw.contains(&expected));
        assert!(!raw.to_ascii_lowercase().contains("\r\nauthorization:"));
        socket
            .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
            .unwrap();
        let mut opaque = [0_u8; 8];
        socket.read_exact(&mut opaque).unwrap();
        assert_eq!(&opaque, b"\x16\x03\x03\x00\x03\xff\x00\xfe");
        socket.write_all(&opaque).unwrap();
        let mut byte = [0];
        assert_eq!(socket.read(&mut byte).unwrap(), 0);
        done_sender.send(()).unwrap();
    });
    let mut socket = connect(&bridge, "fixture.invalid:443", true);
    assert!(headers(&mut socket).starts_with("HTTP/1.1 200 "));
    socket
        .write_all(b"\x16\x03\x03\x00\x03\xff\x00\xfe")
        .unwrap();
    let mut echoed = [0_u8; 8];
    socket.read_exact(&mut echoed).unwrap();
    assert_eq!(&echoed, b"\x16\x03\x03\x00\x03\xff\x00\xfe");
    let mut command = std::process::Command::new("unexecuted-synthetic-native");
    bridge.apply(&route, &mut command);
    assert!(
        command
            .get_args()
            .all(|argument| !argument.to_string_lossy().contains("cilai:"))
    );
    assert!(
        command
            .get_envs()
            .filter_map(|(_, value)| value)
            .all(|value| !value.to_string_lossy().contains(&credential.password))
    );
    allowed.store(false, Ordering::Release);
    done.recv_timeout(Duration::from_secs(3)).unwrap();
    worker.join().unwrap();
    bridge.shutdown_checked().unwrap();
    credential.confirm_cleanup();
}

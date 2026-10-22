//! 原生子进程只持有本机一次性认证；上游代理凭据不会进入其环境。
use super::ResolvedNetwork;
use base64::Engine as _;
use std::fmt::Write as _;
use std::io;
use std::net::TcpListener;
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};

pub(crate) type Admission = Arc<dyn Fn() -> Result<Box<dyn Send>, String> + Send + Sync>;
type Stream = Box<dyn AsyncStream>;
trait AsyncStream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> AsyncStream for T {}

pub(crate) struct NativeProxy {
    endpoint: String,
    stop: mpsc::Sender<()>,
    exited: Mutex<mpsc::Receiver<()>>,
    thread: Mutex<Option<std::thread::JoinHandle<()>>>,
}

struct Context {
    route: ResolvedNetwork,
    authorization: String,
    allowed: Vec<String>,
    admission: Admission,
}

pub(crate) fn allowed_targets(
    kind: &contexthub_protocol::ModelConnectionKind,
    base: Option<&str>,
) -> Result<Vec<String>, String> {
    if *kind == contexthub_protocol::ModelConnectionKind::Chatgpt {
        return Ok([
            "auth.openai.com:443",
            "api.openai.com:443",
            "chatgpt.com:443",
            "ab.chatgpt.com:443",
        ]
        .map(str::to_owned)
        .to_vec());
    }
    let url = reqwest::Url::parse(base.unwrap_or("https://api.openai.com/v1"))
        .map_err(|_| "model_endpoint_invalid")?;
    let host = url.host_str().ok_or("model_endpoint_invalid")?;
    let port = url
        .port_or_known_default()
        .ok_or("model_endpoint_invalid")?;
    Ok(vec![format!("{host}:{port}")])
}

impl NativeProxy {
    pub(crate) fn start(
        route: ResolvedNetwork,
        allowed: Vec<String>,
        admission: Admission,
    ) -> Result<Self, String> {
        if allowed.is_empty() || route.selected_tls_proxy().is_none() {
            return Err("model_native_proxy_route_invalid".into());
        }
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .map_err(|_| "model_native_proxy_start_failed")?;
        listener
            .set_nonblocking(true)
            .map_err(|_| "model_native_proxy_start_failed")?;
        let address = listener
            .local_addr()
            .map_err(|_| "model_native_proxy_start_failed")?;
        let nonce = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        let authorization = format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(format!("cilai:{nonce}"))
        );
        let endpoint = format!("http://cilai:{nonce}@{address}");
        let context = Arc::new(Context {
            route,
            authorization,
            allowed,
            admission,
        });
        let (stop, stopped) = mpsc::channel();
        let (exited_sender, exited) = mpsc::channel();
        let (ready_sender, ready) = mpsc::sync_channel(1);
        let thread = std::thread::Builder::new()
            .name("personal-model-native-proxy".into())
            .spawn(move || {
                let result = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build();
                match result {
                    Ok(runtime) => {
                        runtime.block_on(serve(listener, context, stopped, ready_sender));
                    }
                    Err(_) => {
                        let _ =
                            ready_sender.send(Err("model_native_proxy_start_failed".to_owned()));
                    }
                }
                let _ = exited_sender.send(());
            })
            .map_err(|_| "model_native_proxy_start_failed")?;
        ready
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| "model_native_proxy_start_unconfirmed")??;
        Ok(Self {
            endpoint,
            stop,
            exited: Mutex::new(exited),
            thread: Mutex::new(Some(thread)),
        })
    }

    pub(crate) fn apply(&self, route: &ResolvedNetwork, command: &mut std::process::Command) {
        route.apply_native(command);
        for name in ["HTTPS_PROXY", "https_proxy"] {
            command.env(name, &self.endpoint);
        }
        for name in ["HTTP_PROXY", "http_proxy"] {
            command.env_remove(name);
        }
    }

    pub(crate) fn shutdown_checked(&self) -> Result<(), String> {
        let mut thread = self
            .thread
            .lock()
            .map_err(|_| "model_native_proxy_state_unavailable")?;
        if thread.is_none() {
            return Ok(());
        }
        let _ = self.stop.send(());
        self.exited
            .lock()
            .map_err(|_| "model_native_proxy_state_unavailable")?
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| "model_native_proxy_exit_unconfirmed")?;
        if let Some(thread) = thread.take() {
            thread
                .join()
                .map_err(|_| "model_native_proxy_exit_unconfirmed")?;
        }
        Ok(())
    }
}

impl Drop for NativeProxy {
    fn drop(&mut self) {
        let _ = self.shutdown_checked();
    }
}

async fn serve(
    listener: TcpListener,
    context: Arc<Context>,
    stopped: mpsc::Receiver<()>,
    ready: mpsc::SyncSender<Result<(), String>>,
) {
    let Ok(listener) = tokio::net::TcpListener::from_std(listener) else {
        let _ = ready.send(Err("model_native_proxy_start_failed".into()));
        return;
    };
    if ready.send(Ok(())).is_err() {
        return;
    }
    let mut tasks = tokio::task::JoinSet::new();
    let mut check = tokio::time::interval(Duration::from_millis(50));
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                if let Ok((socket, _)) = accepted {
                    if tasks.len() >= 32 { drop(socket); continue; }
                    let context = Arc::clone(&context);
                    tasks.spawn(async move { let _ = handle(socket, context).await; });
                }
            }
            _ = check.tick() => { if !matches!(stopped.try_recv(), Err(mpsc::TryRecvError::Empty)) { break; } }
            _ = tasks.join_next(), if !tasks.is_empty() => {}
        }
    }
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
}

async fn header(stream: &mut (impl AsyncRead + Unpin)) -> io::Result<Vec<u8>> {
    tokio::time::timeout(Duration::from_secs(10), async {
        let mut value = Vec::new();
        loop {
            if value.len() >= 32_768 {
                return Err(io::Error::other("model_proxy_headers_invalid"));
            }
            value.push(stream.read_u8().await?);
            if value.ends_with(b"\r\n\r\n") {
                return Ok(value);
            }
        }
    })
    .await
    .map_err(|_| io::Error::other("model_proxy_header_timeout"))?
}

fn target(request: &[u8], context: &Context) -> Result<String, ()> {
    let request = std::str::from_utf8(request).map_err(|_| ())?;
    let mut lines = request.split("\r\n");
    let parts = lines.next().ok_or(())?.split(' ').collect::<Vec<_>>();
    if parts.len() != 3
        || parts[0] != "CONNECT"
        || parts[2] != "HTTP/1.1"
        || !context.allowed.iter().any(|value| value == parts[1])
    {
        return Err(());
    }
    let mut authorized = false;
    let mut host = false;
    for line in lines.filter(|line| !line.is_empty()) {
        let (name, value) = line.split_once(':').ok_or(())?;
        match name.to_ascii_lowercase().as_str() {
            "proxy-authorization" => {
                if authorized || !equal(value.trim().as_bytes(), context.authorization.as_bytes()) {
                    return Err(());
                }
                authorized = true;
            }
            "host" => {
                if host || value.trim() != parts[1] {
                    return Err(());
                }
                host = true;
            }
            "authorization" | "cookie" | "transfer-encoding" | "upgrade" => return Err(()),
            "content-length" if value.trim() != "0" => return Err(()),
            _ => {}
        }
    }
    if !authorized || !host {
        return Err(());
    }
    Ok(parts[1].into())
}

fn equal(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .fold(0, |result, (a, b)| result | (a ^ b))
            == 0
}

async fn handle(mut client: tokio::net::TcpStream, context: Arc<Context>) -> io::Result<()> {
    let mut request = header(&mut client).await?;
    let destination = target(&request, &context);
    request.fill(0);
    let Ok(destination) = destination else {
        client.write_all(b"HTTP/1.1 407 Proxy Authentication Required\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await?;
        return Ok(());
    };
    let _admission = (context.admission)()
        .map_err(|_| io::Error::other("model_native_proxy_admission_refused"))?;
    context
        .route
        .validate_credentials()
        .map_err(|_| io::Error::other("model_network_credentials_changed"))?;
    let mut upstream = connect(&context.route).await?;
    let mut forwarded = format!("CONNECT {destination} HTTP/1.1\r\nHost: {destination}\r\n");
    if let Some(basic) = context.route.proxy_basic() {
        let encoded = base64::engine::general_purpose::STANDARD
            .encode(format!("{}:{}", basic.username, basic.password));
        write!(forwarded, "Proxy-Authorization: Basic {encoded}\r\n")
            .map_err(|_| io::Error::other("model_native_proxy_auth_invalid"))?;
    }
    forwarded.push_str("\r\n");
    let mut forwarded = forwarded.into_bytes();
    let sent = upstream.write_all(&forwarded).await;
    forwarded.fill(0);
    sent?;
    let response = header(&mut upstream).await?;
    if !response.starts_with(b"HTTP/1.1 200 ") && !response.starts_with(b"HTTP/1.0 200 ") {
        let response: &[u8] = if response.starts_with(b"HTTP/1.1 407 ")
            || response.starts_with(b"HTTP/1.0 407 ")
        {
            b"HTTP/1.1 407 Proxy Authentication Required\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        } else {
            b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        };
        client.write_all(response).await?;
        return Ok(());
    }
    client
        .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
        .await?;
    let copy = tokio::io::copy_bidirectional(&mut client, &mut upstream);
    tokio::pin!(copy);
    let mut check = tokio::time::interval(Duration::from_millis(200));
    loop {
        tokio::select! {
            copied = &mut copy => { copied?; return Ok(()); }
            _ = check.tick() => {
                let _checked = (context.admission)().map_err(|_| io::Error::other("model_native_proxy_scope_revoked"))?;
                context.route.validate_credentials().map_err(|_| io::Error::other("model_network_credentials_changed"))?;
            }
        }
    }
}

async fn connect(route: &ResolvedNetwork) -> io::Result<Stream> {
    let proxy = reqwest::Url::parse(
        route
            .selected_tls_proxy()
            .ok_or_else(|| io::Error::other("model_native_proxy_route_invalid"))?,
    )
    .map_err(|_| io::Error::other("model_native_proxy_route_invalid"))?;
    let host = proxy
        .host_str()
        .ok_or_else(|| io::Error::other("model_native_proxy_route_invalid"))?;
    let port = proxy
        .port_or_known_default()
        .ok_or_else(|| io::Error::other("model_native_proxy_route_invalid"))?;
    let socket = tokio::time::timeout(
        Duration::from_secs(15),
        tokio::net::TcpStream::connect((host, port)),
    )
    .await
    .map_err(|_| io::Error::other("model_native_proxy_connect_timeout"))??;
    if proxy.scheme() == "http" {
        return Ok(Box::new(socket));
    }
    let config = tls_configuration(route)?;
    let name = rustls::pki_types::ServerName::try_from(host.to_owned())
        .map_err(|_| io::Error::other("model_native_proxy_hostname_invalid"))?;
    let tls = tokio::time::timeout(
        Duration::from_secs(15),
        tokio_rustls::TlsConnector::from(Arc::new(config)).connect(name, socket),
    )
    .await
    .map_err(|_| io::Error::other("model_native_proxy_tls_timeout"))??;
    Ok(Box::new(tls))
}

fn tls_configuration(route: &ResolvedNetwork) -> io::Result<rustls::ClientConfig> {
    use rustls::pki_types::pem::PemObject as _;
    let mut roots = rustls::RootCertStore::empty();
    roots.add_parsable_certificates(rustls_native_certs::load_native_certs().certs);
    if let Some(pem) = route.ca_certificate() {
        for certificate in rustls::pki_types::CertificateDer::pem_slice_iter(pem.as_bytes()) {
            roots
                .add(certificate.map_err(|_| io::Error::other("model_ca_certificate_invalid"))?)
                .map_err(|_| io::Error::other("model_ca_certificate_invalid"))?;
        }
    }
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::aws_lc_rs::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|_| io::Error::other("model_native_proxy_tls_failed"))?
    .with_root_certificates(roots)
    .with_no_client_auth();
    Ok(config)
}

#[cfg(test)]
mod tests;

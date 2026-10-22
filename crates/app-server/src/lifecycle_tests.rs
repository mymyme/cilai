use super::*;
use contexthub_protocol::Frame;
use serde_json::{Value, json};

fn request(router: &mut crate::ConnectionRouter, method: &str) -> (crate::RouteResult, Value) {
    let response = router.handle_frame(Frame::Message(
        json!({"jsonrpc":"2.0","id":1,"method":method,"params":{}})
            .to_string()
            .into_bytes(),
    ));
    let value = serde_json::from_str(response.response.as_ref().unwrap()).unwrap();
    (response, value)
}
#[test]
fn new_peer_between_prepare_and_execute_is_kept_and_shutdown_admission_is_atomic() {
    let lifecycle = AppServerLifecycle::new(true);
    let mut router = crate::ConnectionRouter::new("test-server".into())
        .with_lifecycle(lifecycle.clone(), LifecycleCaller::Stdio);
    router.phase = crate::Phase::Ready;
    let (_, prepared) = request(&mut router, "shutdown/prepare");
    assert_eq!(prepared["result"]["ready"], true);
    let peer = lifecycle.admit_ipc().unwrap().unwrap();
    let (blocked, value) = request(&mut router, "shutdown/execute");
    assert!(!blocked.should_exit);
    assert_eq!(value["result"]["accepted"], false);
    assert_eq!(value["result"]["continuing"], true);
    assert_eq!(
        value["result"]["reasonCode"],
        "app_server_shared_clients_active"
    );
    assert_eq!(value["result"]["serverInstanceId"], "test-server");
    assert_eq!(value["result"]["ipcClientCount"], 1);
    assert_eq!(router.phase, crate::Phase::Ready);
    drop(peer);
    let (_, prepared) = request(&mut router, "shutdown/prepare");
    assert_eq!(prepared["result"]["ready"], true);
    let (accepted, value) = request(&mut router, "shutdown/execute");
    assert!(accepted.should_exit);
    assert_eq!(value["result"]["accepted"], true);
    assert_eq!(value["result"]["continuing"], false);
    assert!(lifecycle.admit_ipc().unwrap().is_none());
    assert!(lifecycle.begin_detached_request().is_err());
}
#[test]
fn stdio_eof_preserves_real_peers_and_detached_work_until_both_end() {
    let lifecycle = AppServerLifecycle::new(true);
    let peer = lifecycle.admit_ipc().unwrap().unwrap();
    let work = lifecycle.begin_detached_request().unwrap();
    lifecycle.close_stdio().unwrap();
    let continued = lifecycle
        .shutdown_decision(LifecycleCaller::Disconnected, None, true)
        .unwrap();
    assert!(continued.continuing);
    assert!(!continued.can_close);
    drop(peer);
    let running = lifecycle
        .shutdown_decision(LifecycleCaller::Disconnected, None, true)
        .unwrap();
    assert!(!running.can_close);
    assert!(!running.continuing);
    assert_eq!(
        running.blocking_reasons,
        vec!["app_server_requests_active_or_unconfirmed"]
    );
    drop(work);
    assert!(
        lifecycle
            .shutdown_decision(LifecycleCaller::Disconnected, None, true)
            .unwrap()
            .can_close
    );
    assert!(lifecycle.admit_ipc().unwrap().is_none());
}
#[test]
fn an_ipc_client_cannot_shutdown_the_existing_stdio_owner() {
    let lifecycle = AppServerLifecycle::new(true);
    let _client = lifecycle.admit_ipc().unwrap().unwrap();
    let blocked = lifecycle
        .shutdown_decision(LifecycleCaller::Ipc, None, true)
        .unwrap();
    assert!(blocked.continuing);
    assert!(!blocked.can_close);
    assert_eq!(blocked.ipc_client_count, 1);
}
struct BrokenWriter;
impl tokio::io::AsyncWrite for BrokenWriter {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
        _: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        std::task::Poll::Ready(Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe)))
    }
    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }
    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }
}
#[test]
fn accepted_shutdown_still_notifies_owner_if_the_response_pipe_has_closed() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let lifecycle = AppServerLifecycle::new(true);
            let mut router = crate::ConnectionRouter::new("test-server".into())
                .with_lifecycle(lifecycle.clone(), LifecycleCaller::Stdio);
            router.phase = crate::Phase::ShutdownPrepared;
            let data =
                b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"shutdown/execute\",\"params\":{}}\n";
            let (tx, rx) = tokio::sync::watch::channel(false);
            let observer = rx.clone();
            assert!(
                crate::serve_connection(&data[..], BrokenWriter, router, tx, rx)
                    .await
                    .is_err()
            );
            assert!(*observer.borrow());
            assert!(lifecycle.admit_ipc().unwrap().is_none());
        });
}

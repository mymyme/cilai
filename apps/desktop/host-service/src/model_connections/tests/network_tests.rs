use super::{Cell, ModelCredentialAction, RefCell, TestStore, prepared};
use crate::model_connections::network_credentials;
use contexthub_protocol::{
    MODEL_NETWORK_CREDENTIAL_SERVICE, ModelNetworkAuthentication, ModelNetworkCredentials,
    ModelNetworkMode, ModelSecretRef,
};

#[test]
fn network_bundle_is_written_exactly_once_and_pending_value_cannot_be_overwritten() {
    let mut change = prepared();
    change.network_credential_action = ModelCredentialAction::Replace;
    let network = &mut change.connection.config.network;
    network.mode = ModelNetworkMode::CustomProxy;
    network.proxy_auth = ModelNetworkAuthentication::Basic;
    network.proxy_url = Some("http://127.0.0.1:8080".into());
    network.header_names = vec!["x-service-key".into()];
    network.proxy_secret_ref = Some(ModelSecretRef {
        service: MODEL_NETWORK_CREDENTIAL_SERVICE.into(),
        account: format!(
            "{}:{}:1",
            change.connection.profile_id, change.connection.connection_id
        ),
    });
    let secret: ModelNetworkCredentials = serde_json::from_value(serde_json::json!({"proxyBasic":{"username":"synthetic-user","password":"synthetic-password"},"serviceBasic":null,"headers":{"x-service-key":"synthetic-header-secret"}})).unwrap();
    let store = TestStore {
        value: RefCell::new(Vec::new()),
        writes: Cell::new(0),
        ..Default::default()
    };
    for _ in 0..2 {
        network_credentials::write(
            &change.connection.profile_id,
            &change,
            Some(&secret),
            &store,
        )
        .unwrap();
    }
    assert_eq!(store.writes.get(), 1);
    let other: ModelNetworkCredentials = serde_json::from_value(serde_json::json!({"proxyBasic":{"username":"synthetic-user","password":"changed"},"serviceBasic":null,"headers":{"x-service-key":"synthetic-header-secret"}})).unwrap();
    assert_eq!(
        network_credentials::write(&change.connection.profile_id, &change, Some(&other), &store)
            .unwrap_err(),
        "model_network_credential_pending_value_conflict"
    );
    assert_eq!(store.writes.get(), 1);
    assert!(
        !serde_json::to_string(&change)
            .unwrap()
            .contains("synthetic-header-secret")
    );
}

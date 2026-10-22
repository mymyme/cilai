use super::{Fixture, PROFILE, Value, json};

fn request(operation: &str) -> Value {
    json!({"operationId":operation,"config":{"name":"合成网络认证","kind":"custom_responses",
        "baseUrl":"https://fixture.invalid/v1","authMode":"none","enabled":true,
        "network":{"mode":"custom_proxy","proxyUrl":"http://proxy.invalid:8080",
            "proxyAuth":"basic","headerNames":["x-service-key"]}},
        "credentialAction":"clear","networkCredentialAction":"replace"})
}

fn commit(f: &mut Fixture, prepared: &Value) -> Value {
    f.call(
        "modelConnection/saveCommit",
        json!({"operationId":prepared["operationId"],
        "networkCredentialRef":prepared["connection"]["config"]["network"]["proxySecretRef"]}),
    )
}

fn candidate(f: &mut Fixture, reference: &Value) -> Value {
    f.call("modelCredentialCleanup/list", json!({}))
        .as_array()
        .unwrap()
        .iter()
        .find(|candidate| candidate["credentialRef"] == *reference)
        .unwrap()
        .clone()
}

#[test]
fn network_credentials_are_separate_durable_and_never_reuse_an_aborted_generation() {
    let mut f = Fixture::new();
    let first = f.call("modelConnection/savePrepare", request("network-first"));
    let conn = &first["connection"];
    let reference = &conn["config"]["network"]["proxySecretRef"];
    assert_eq!(
        reference["service"],
        contexthub_protocol::MODEL_NETWORK_CREDENTIAL_SERVICE
    );
    assert_eq!(
        reference["account"],
        format!("{PROFILE}:{}:1", conn["connectionId"].as_str().unwrap())
    );
    assert!(conn["secretRef"].is_null());
    f.reject(
        "modelConnection/saveCommit",
        json!({"operationId":"network-first"}),
        "model_credential_receipt_mismatch",
    );
    f.call(
        "modelConnection/saveAbort",
        json!({"operationId":"network-first"}),
    );
    let mut second_request = request("network-second");
    second_request["connectionId"] = conn["connectionId"].clone();
    let second = f.call("modelConnection/savePrepare", second_request);
    assert_eq!(second["connection"]["configRevision"], 2);
    assert_ne!(
        second["connection"]["config"]["network"]["proxySecretRef"],
        *reference
    );
    let saved = commit(&mut f, &second);
    let reopened = f.call(
        "modelConnection/get",
        json!({"connectionId":saved["connectionId"]}),
    );
    assert_eq!(reopened, saved);
    let orphan = candidate(&mut f, reference);
    assert_eq!(orphan["canPrepare"], true);
    let plan = f.call("modelCredentialCleanup/prepare",json!({"cleanupId":orphan["cleanupId"],"expectedReferenceDigest":orphan["referenceDigest"]}));
    let ack = json!({"cleanupId":plan["cleanupId"],"expectedReferenceDigest":plan["referenceDigest"],"credentialRef":reference});
    let confirmed = f.call("modelCredentialCleanup/ack", ack.clone());
    assert_eq!(confirmed["state"], "confirmed");
    assert_eq!(f.call("modelCredentialCleanup/ack", ack), confirmed);
    assert_eq!(
        candidate(&mut f, &saved["config"]["network"]["proxySecretRef"])["canPrepare"],
        false
    );
}

#[test]
fn changing_route_or_declarations_requires_explicit_network_rotation_and_rejects_foreign_refs() {
    let mut f = Fixture::new();
    let prepared = f.call("modelConnection/savePrepare", request("network-original"));
    let saved = commit(&mut f, &prepared);
    let mut edit = json!({"operationId":"network-edit","connectionId":saved["connectionId"],
        "expectedConfigRevision":saved["configRevision"],"config":saved["config"],
        "credentialAction":"keep","networkCredentialAction":"keep"});
    edit["config"]["network"]["proxyUrl"] = json!("http://other-proxy.invalid:8080");
    f.reject(
        "modelConnection/savePrepare",
        edit.clone(),
        "model_network_credential_rotation_required",
    );
    edit["networkCredentialAction"] = json!("replace");
    edit["config"]["network"]["proxySecretRef"]["account"] =
        json!(format!("{}:{}:1", "2".repeat(32), "3".repeat(32)));
    f.reject(
        "modelConnection/savePrepare",
        edit,
        "model_proxy_secret_scope_invalid",
    );
    let mut clear = json!({"operationId":"network-clear","connectionId":saved["connectionId"],"expectedConfigRevision":saved["configRevision"],"config":saved["config"],"credentialAction":"keep","networkCredentialAction":"clear"});
    f.reject(
        "modelConnection/savePrepare",
        clear.clone(),
        "model_network_credential_required",
    );
    clear["config"]["network"]["proxyAuth"] = json!("none");
    clear["config"]["network"]["headerNames"] = json!([]);
    let cleared = f.call("modelConnection/savePrepare", clear);
    let cleared = commit(&mut f, &cleared);
    assert!(cleared["config"]["network"]["proxySecretRef"].is_null());
    assert_eq!(
        candidate(&mut f, &saved["config"]["network"]["proxySecretRef"])["canPrepare"],
        true
    );
}

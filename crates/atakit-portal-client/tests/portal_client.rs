//! Contract tests over real HTTP/Unix sockets, following Portal's workload routes.
use alloy::primitives::B256;
use atakit_portal_client as portal;
use axum::{
    Router,
    body::{Body, Bytes, to_bytes},
    extract::{Request, State},
    response::Response,
};
use portal::{BabyCreateRequest, ClientError, PortalClient};
use serde_json::{Value, json};
use std::{collections::VecDeque, sync::Arc, time::Duration};
use tokio::{net::UnixListener, sync::Mutex};

struct Expected {
    method: &'static str,
    uri: String,
    body: Value,
    response: Value,
    status: u16,
}
struct Mock {
    client: PortalClient,
    task: tokio::task::JoinHandle<()>,
    queue: Arc<Mutex<VecDeque<Expected>>>,
    _dir: tempfile::TempDir,
}
impl Drop for Mock {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Mock {
    async fn new(expected: Vec<Expected>) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let queue = Arc::new(Mutex::new(VecDeque::from(expected)));
        async fn serve(
            State(queue): State<Arc<Mutex<VecDeque<Expected>>>>,
            req: Request,
        ) -> Response {
            let exp = queue
                .lock()
                .await
                .pop_front()
                .expect("unexpected request (possibly a retry)");
            assert_eq!(req.method().as_str(), exp.method);
            assert_eq!(req.uri().to_string(), exp.uri);
            let content_type = req
                .headers()
                .get("content-type")
                .map(|s| s.to_str().unwrap().to_owned());
            let bytes = to_bytes(req.into_body(), 2 * 1024 * 1024).await.unwrap();
            if exp.body == json!("binary") {
                assert_eq!(content_type.as_deref(), Some("application/octet-stream"));
                assert_eq!(bytes.as_ref(), &[0, 255, 13, 10]);
            } else if exp.body.is_null() {
                assert!(bytes.is_empty());
            } else {
                assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap(), exp.body);
            }
            Response::builder()
                .status(exp.status)
                .header("Content-Type", "application/json")
                .header("Retry-After", "2")
                .body(Body::from(if exp.status == 204 {
                    String::new()
                } else {
                    exp.response.to_string()
                }))
                .unwrap()
        }
        let router = Router::new().fallback(serve).with_state(queue.clone());
        let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        Self {
            client: PortalClient::new(path),
            task,
            queue,
            _dir: dir,
        }
    }
    async fn consumed(&self) {
        assert!(self.queue.lock().await.is_empty());
        assert!(!self.task.is_finished());
    }
}
fn get(uri: &str, response: Value) -> Expected {
    Expected {
        method: "GET",
        uri: uri.into(),
        body: Value::Null,
        response,
        status: 200,
    }
}
fn post(uri: &str, body: Value, response: Value, status: u16) -> Expected {
    Expected {
        method: "POST",
        uri: uri.into(),
        body,
        response,
        status,
    }
}

#[tokio::test]
async fn reads_status_using_portal_schema() {
    let status: Value = serde_json::from_str(include_str!("status-response.json")).unwrap();
    let mock = Mock::new(vec![get("/portal-external-api/status", status.clone())]).await;
    let actual = mock.client.status().await.unwrap();
    assert_eq!(actual.state, "Running");
    assert_eq!(serde_json::to_value(actual).unwrap(), status);
    mock.consumed().await;
}

#[tokio::test]
async fn signs_with_optional_session_binding_and_checks_signature() {
    let fixture: Value = serde_json::from_str(include_str!("sign-response.json")).unwrap();
    let session: B256 = fixture["session_id"].as_str().unwrap().parse().unwrap();
    let mock = Mock::new(vec![
        post(
            "/sign-message",
            json!({"message":"0x68656c6c6f","hash_fn":"keccak256"}),
            fixture.clone(),
            200,
        ),
        post(
            "/sign-message",
            json!({"message":"0x68656c6c6f","hash_fn":"keccak256","expected_session_id":session}),
            fixture.clone(),
            200,
        ),
        post(
            "/sign-message",
            json!({"message":"0x626164","hash_fn":"keccak256"}),
            fixture,
            200,
        ),
    ])
    .await;
    mock.client.sign_message(b"hello").await.unwrap();
    mock.client
        .sign_message_for_session(b"hello", session)
        .await
        .unwrap();
    assert!(matches!(
        mock.client.sign_message(b"bad").await,
        Err(ClientError::InvalidSignature(_))
    ));
    mock.consumed().await;
}

#[tokio::test]
async fn exposes_http_status_retry_after_and_raw_body_without_retrying_mutations() {
    let err = json!({"code":"session_not_ready","retryable":true});
    let mock = Mock::new(vec![post(
        "/sign-message",
        json!({"message":"0x","hash_fn":"keccak256"}),
        err.clone(),
        503,
    )])
    .await;
    let error = mock.client.sign_message(b"").await.unwrap_err();
    let response = error.http_response().unwrap();
    assert_eq!(response.status.as_u16(), 503);
    assert_eq!(error.retry_after(), Some("2"));
    assert_eq!(
        serde_json::from_slice::<Value>(&response.body).unwrap(),
        err
    );
    mock.consumed().await;
}

#[tokio::test]
async fn supports_every_baby_container_route_and_binary_upload() {
    let image =
        json!({"image_id":"sha256:abc","slot":"worker","uploaded_by":"app","retention":"session"});
    let instance = json!({"instance_id":"one","slot":"worker","image_id":"sha256:abc","container_id":"c1","container_name":"baby-one","parent_service":"app","uid_slot":0,"cap_add":["NET_ADMIN"],"status":"running"});
    let mock = Mock::new(vec![
        post("/baby-container/image/upload?slot=worker",json!("binary"),json!({"image_id":"sha256:abc"}),200),
        get("/baby-container/image/list?slot=a%20%26b",json!({"images":[image.clone()]})),
        get("/baby-container/image/list",json!({"images":[]})),
        post("/baby-container/create",json!({"slot":"worker","image_id":"sha256:abc","instance_id":"one","cap-add":["NET_ADMIN"]}),json!({"instance":instance.clone()}),200),
        get("/baby-container/list",json!({"images":[image],"instances":[instance]})),
        get("/baby-container/logs?instance_id=a%2Fb%3Fc%26d&max_bytes=4096",json!({"logs":"hello\n"})),
        post("/baby-container/stop",json!({"instance_id":"one"}),Value::Null,204),
        post("/baby-container/remove",json!({"instance_id":"one"}),Value::Null,204),
        post("/baby-container/image/remove",json!({"image_id":"sha256:abc","slot":"worker"}),Value::Null,204),
    ]).await;
    assert_eq!(
        mock.client
            .upload_baby_image("worker", Bytes::from_static(&[0, 255, 13, 10]))
            .await
            .unwrap()
            .image_id,
        "sha256:abc"
    );
    assert_eq!(
        mock.client
            .list_baby_images(Some("a &b"))
            .await
            .unwrap()
            .images
            .len(),
        1
    );
    assert!(
        mock.client
            .list_baby_images(None)
            .await
            .unwrap()
            .images
            .is_empty()
    );
    let req = BabyCreateRequest {
        slot: "worker".into(),
        image_id: Some("sha256:abc".into()),
        instance_id: Some("one".into()),
        cap_add: vec!["NET_ADMIN".into()],
    };
    assert_eq!(
        mock.client
            .create_baby_container(&req)
            .await
            .unwrap()
            .instance
            .instance_id,
        "one"
    );
    assert_eq!(
        mock.client
            .list_baby_containers()
            .await
            .unwrap()
            .instances
            .len(),
        1
    );
    assert_eq!(
        mock.client
            .baby_container_logs("a/b?c&d", Some(4096))
            .await
            .unwrap()
            .logs,
        "hello\n"
    );
    mock.client.stop_baby_container("one").await.unwrap();
    mock.client.remove_baby_container("one").await.unwrap();
    mock.client
        .remove_baby_image("sha256:abc", Some("worker"))
        .await
        .unwrap();
    mock.consumed().await;
}

#[tokio::test]
async fn bounds_response_size_and_reports_malformed_success() {
    let mock = Mock::new(vec![get(
        "/portal-external-api/status",
        json!({"state":"Running","padding":"x".repeat(256)}),
    )])
    .await;
    assert!(
        mock.client
            .clone()
            .with_response_limit(32)
            .status()
            .await
            .is_err()
    );
    let mock = Mock::new(vec![get(
        "/portal-external-api/status",
        json!({"unexpected":true}),
    )])
    .await;
    assert!(matches!(
        mock.client.status().await,
        Err(ClientError::Decode(_))
    ));
}

#[tokio::test]
async fn request_timeout_covers_waiting_for_response_headers() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hang.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let server = tokio::spawn(async move {
        let (_conn, _) = listener.accept().await.unwrap();
        tokio::time::sleep(Duration::from_secs(2)).await;
    });
    let error = PortalClient::new(path)
        .with_timeout(Duration::from_millis(30))
        .status()
        .await
        .unwrap_err();
    assert!(matches!(error, ClientError::Timeout));
    server.abort();
}

fn evidence_fixture() -> Value {
    serde_json::from_str(include_str!("evidence-bundle.json")).unwrap()
}
const CHALLENGE_QUERY: &str = "/portal-external-api/session/evidence-bundle?challenge=QkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkI";
#[tokio::test]
async fn obtains_owner_public_key_over_socket_without_session_id_or_chain_rpc() {
    let fixture = evidence_fixture();
    let owner: B256 = fixture["evidence_bundle"]["owner"]["fingerprint"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let mock = Mock::new(vec![get(CHALLENGE_QUERY, fixture)]).await;
    let key = mock
        .client
        .owner_public_key(owner, [0x42; 32])
        .await
        .unwrap();
    assert_eq!(key.fingerprint, owner);
    assert_eq!(key.type_id, 3);
    assert_eq!(
        alloy::hex::encode_prefixed(key.key),
        "0x0479be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798483ada7726a3c4655da4fbfc0e1108a8fd17b448a68554199c47d08ffb10d4b8"
    );
    mock.consumed().await;
}
#[test]
fn evidence_binding_rejects_replay_tampering_and_wrong_session_key() {
    let fixture = evidence_fixture();
    let bundle: portal::EvidenceBundle = serde_json::from_value(fixture.clone()).unwrap();
    bundle.validate_binding([0x42; 32]).unwrap();
    assert!(bundle.validate_binding([0x43; 32]).is_err());
    let mut bad = bundle.clone();
    mutate_evidence(&mut bad, "/binding/chain_id", json!(1));
    assert!(bad.validate_binding([0x42; 32]).is_err());
    let mut bad = bundle.clone();
    bad.request_binding.signature = alloy::hex::encode_prefixed([0; 65]);
    assert!(bad.validate_binding([0x42; 32]).is_err());
    let mut bad = bundle.clone();
    mutate_evidence(&mut bad, "/session_key/fingerprint", json!(B256::ZERO));
    assert!(bad.validate_binding([0x42; 32]).is_err());
}
#[test]
fn owner_recovery_requires_matching_trusted_fingerprint_and_authorization() {
    let mut bundle: portal::EvidenceBundle = serde_json::from_value(evidence_fixture()).unwrap();
    assert!(bundle.owner_public_key(B256::ZERO).is_err());
    let owner: B256 = bundle.claims().unwrap().owner.fingerprint.parse().unwrap();
    mutate_evidence(
        &mut bundle,
        "/owner/contract_authorization/payload",
        json!(B256::ZERO),
    );
    assert!(bundle.owner_public_key(owner).is_err());
    mutate_evidence(&mut bundle, "/owner/contract_authorization", Value::Null);
    assert!(bundle.owner_public_key(owner).is_err());
}

// Deliberately edit arbitrary wire fields to exercise malformed evidence.
fn mutate_evidence(bundle: &mut portal::EvidenceBundle, pointer: &str, value: Value) {
    *bundle.evidence_bundle.pointer_mut(pointer).unwrap() = value;
}

#[test]
fn status_matches_required_fields_and_enums_in_portal() {
    let fixture: Value = serde_json::from_str(include_str!("status-response.json")).unwrap();
    let status: portal::PortalStatus = serde_json::from_value(fixture.clone()).unwrap();
    assert_eq!(status.chain.chain_id, Some(560048));
    assert_eq!(status.chain.status, "verified");
    assert_eq!(
        status.platform.cloud_provenance.source,
        portal::CloudSource::UserProvided
    );
    for pointer in ["/init_schema_version", "/started_at", "/platform", "/chain"] {
        let mut bad = fixture.clone();
        bad.as_object_mut().unwrap().remove(&pointer[1..]);
        assert!(
            serde_json::from_value::<portal::PortalStatus>(bad).is_err(),
            "{pointer}"
        );
    }
    for (pointer, value) in [
        ("/chain/chain_id", json!("invalid")),
        ("/chain/registration", Value::Null),
        ("/platform/cloud_provenance/source", json!("invented")),
        (
            "/platform/cloud_provenance/detection/metadata/conflict",
            Value::Null,
        ),
    ] {
        let mut bad = fixture.clone();
        *bad.pointer_mut(pointer).unwrap() = value;
        assert!(
            serde_json::from_value::<portal::PortalStatus>(bad).is_err(),
            "{pointer}"
        );
    }
    let mut extended = fixture.clone();
    extended["chain"]["future_flag"] = json!({"enabled":true});
    let status: portal::PortalStatus = serde_json::from_value(extended).unwrap();
    assert_eq!(serde_json::to_value(status).unwrap(), fixture);
}

#[test]
fn evidence_preserves_unknown_signed_fields() {
    let mut wire = evidence_fixture();
    wire["evidence_bundle"]["future_field"] = json!({"nested":[1,"two"]});
    let original = wire["evidence_bundle"].clone();
    let bundle: portal::EvidenceBundle = serde_json::from_value(wire).unwrap();
    assert_eq!(bundle.evidence_bundle, original);
    assert_eq!(bundle.claims().unwrap().binding.chain_id, 560048);
    // Added fields must affect the signature even if the typed view ignores them.
    assert!(bundle.validate_binding([0x42; 32]).is_err());
}

#[test]
fn evidence_binding_uses_canonical_json_and_rejects_invalid_signature_text() {
    let fixture = evidence_fixture();
    let pretty = serde_json::to_string_pretty(&fixture).unwrap();
    let mut bundle: portal::EvidenceBundle = serde_json::from_str(&pretty).unwrap();
    bundle.validate_binding([0x42; 32]).unwrap();
    let compact = serde_json::to_string(&bundle).unwrap();
    serde_json::from_str::<portal::EvidenceBundle>(&compact)
        .unwrap()
        .validate_binding([0x42; 32])
        .unwrap();
    bundle.request_binding.signature = "not-hex".into();
    assert!(matches!(
        bundle.validate_binding([0x42; 32]),
        Err(ClientError::InvalidSignature(_))
    ));
}

#[tokio::test]
async fn signs_with_sha256_and_rejects_algorithm_substitution() {
    use alloy::signers::{SignerSync, local::PrivateKeySigner};
    use sha2::{Digest, Sha256};
    let mut fixture: Value = serde_json::from_str(include_str!("sign-response.json")).unwrap();
    let digest = B256::from_slice(&Sha256::digest([portal::SIGN_DOMAIN, b"hello"].concat()));
    let signer = PrivateKeySigner::from_bytes(&B256::with_last_byte(1)).unwrap();
    fixture["hash_fn"] = json!("sha256");
    fixture["message_hash"] = json!(digest);
    fixture["signature"] = json!(signer.sign_hash_sync(&digest).unwrap().to_string());
    let expected = B256::repeat_byte(1);
    fixture["session_id"] = json!(expected);
    let request =
        json!({"message":"0x68656c6c6f","hash_fn":"sha256","expected_session_id":expected});
    let mut wrong = fixture.clone();
    wrong["hash_fn"] = json!("keccak256");
    let mock = Mock::new(vec![
        post("/sign-message", request.clone(), fixture, 200),
        post("/sign-message", request, wrong, 200),
    ])
    .await;
    let signed = mock
        .client
        .sign_message_with_hash(b"hello", portal::HashFunction::Sha256, Some(expected))
        .await
        .unwrap();
    assert_eq!(signed.message_hash, digest);
    assert!(
        mock.client
            .sign_message_with_hash(b"hello", portal::HashFunction::Sha256, Some(expected))
            .await
            .is_err()
    );
    mock.consumed().await;
}

#[test]
fn wire_requests_and_evidence_follow_portal_deserialization_rules() {
    use portal::wire::{SessionBinding, SessionPublicKey, SignMessageRequest};
    let request: SignMessageRequest = serde_json::from_str(r#"{"message":"0x"}"#).unwrap();
    assert!(request.hash_fn.is_none());
    assert!(request.expected_session_id.is_none());
    for id in [
        Value::Null,
        json!("0x12"),
        json!(format!("0x{}", "AA".repeat(32))),
    ] {
        assert!(
            serde_json::from_value::<SignMessageRequest>(
                json!({"message":"0x", "expected_session_id":id})
            )
            .is_err()
        );
    }
    let fixture = evidence_fixture();
    let binding = fixture["evidence_bundle"]["binding"].clone();
    serde_json::from_value::<SessionBinding>(binding.clone()).unwrap();
    for name in ["registry", "owner_nonce", "qualifying_data"] {
        let mut bad = binding.clone();
        bad.as_object_mut().unwrap().remove(name);
        assert!(serde_json::from_value::<SessionBinding>(bad).is_err());
    }
    let mut key = fixture["evidence_bundle"]["session_key"].clone();
    key["extra"] = json!(true);
    assert!(serde_json::from_value::<SessionPublicKey>(key).is_err());
}

#[tokio::test]
async fn signs_through_a_long_rootfs_socket_symlink() {
    let fixture: Value = serde_json::from_str(include_str!("sign-response.json")).unwrap();
    let mock = Mock::new(vec![post(
        "/sign-message",
        json!({"message":"0x68656c6c6f","hash_fn":"keccak256"}),
        fixture,
        200,
    )])
    .await;
    let root = tempfile::tempdir().unwrap();
    let directory = root
        .path()
        .join("workload-project-".repeat(10))
        .join("native-root/run");
    std::fs::create_dir_all(&directory).unwrap();
    let socket = directory.join("atakit-portal.sock");
    std::os::unix::fs::symlink(mock.client.socket(), &socket).unwrap();
    assert!(socket.as_os_str().len() > 108);
    let client = PortalClient::new(&socket);
    client.sign_message(b"hello").await.unwrap();
    assert_eq!(client.socket(), socket);
    mock.consumed().await;
}

#[tokio::test]
async fn readiness_timeout_includes_retry_after_delay() {
    let mock = Mock::new(vec![post(
        "/sign-message",
        json!({"message":"0x68656c6c6f","hash_fn":"keccak256"}),
        json!({"code":"session_not_ready"}),
        503,
    )])
    .await;
    let client = mock.client.clone().with_timeout(Duration::from_millis(100));
    let error = tokio::time::timeout(
        Duration::from_secs(1),
        client.sign_message_when_ready(b"hello", portal::HashFunction::Keccak256, None),
    )
    .await
    .expect("the timeout must include the retry delay")
    .unwrap_err();
    assert!(matches!(error, ClientError::Timeout));
    mock.consumed().await;
}

#[test]
fn public_identity_preserves_application_json_and_accepts_portal_field_names() {
    let identity: portal::PublicIdentity =
        serde_json::from_value(json!({"type_id":3,"key":"0x0401"})).unwrap();
    let application_json = json!({"typeId":3,"key":"0x0401"});
    assert_eq!(serde_json::to_value(&identity).unwrap(), application_json);
    assert_eq!(
        serde_json::from_value::<portal::PublicIdentity>(application_json).unwrap(),
        identity
    );
}

#[tokio::test]
async fn owner_recovery_is_bound_to_the_signing_session() {
    let fixture = evidence_fixture();
    let session_id: B256 = fixture["evidence_bundle"]["session_id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let key = &fixture["evidence_bundle"]["session_key"];
    let key = portal::PublicKey {
        type_id: 3,
        key: key["bytes"].as_str().unwrap().parse().unwrap(),
        fingerprint: key["fingerprint"].as_str().unwrap().parse().unwrap(),
    };
    for mismatch in ["none", "session", "key"] {
        let mock = Mock::new(vec![get(CHALLENGE_QUERY, fixture.clone())]).await;
        let signing_key = portal::PublicKey {
            type_id: key.type_id,
            key: key.key.clone(),
            fingerprint: if mismatch == "key" {
                B256::ZERO
            } else {
                key.fingerprint
            },
        };
        let result = mock
            .client
            .owner_public_key_for_session(
                if mismatch == "session" {
                    B256::ZERO
                } else {
                    session_id
                },
                &signing_key,
                Some([0x42; 32]),
            )
            .await;
        if mismatch == "none" {
            let owner = result.unwrap();
            assert_eq!(
                owner.fingerprint.to_string(),
                fixture["evidence_bundle"]["owner"]["fingerprint"]
            );
            assert_ne!(owner.key, key.key);
        } else {
            assert!(result.is_err(), "accepted mismatched {mismatch}");
        }
        mock.consumed().await;
    }
}

#[tokio::test]
async fn automatic_challenges_are_fresh_and_reject_replayed_evidence() {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};

    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("p.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let challenges = Arc::new(Mutex::new(Vec::new()));
    let captured = challenges.clone();
    let router = Router::new().fallback(move |req: Request| {
        let captured = captured.clone();
        async move {
            assert_eq!(
                req.uri().path(),
                "/portal-external-api/session/evidence-bundle"
            );
            let challenge = URL_SAFE_NO_PAD
                .decode(
                    req.uri()
                        .query()
                        .unwrap()
                        .strip_prefix("challenge=")
                        .unwrap(),
                )
                .unwrap();
            assert_eq!(challenge.len(), 32);
            captured.lock().await.push(challenge);
            axum::Json(evidence_fixture())
        }
    });
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let client = PortalClient::new(socket);
    let fixture = evidence_fixture();
    let key = &fixture["evidence_bundle"]["session_key"];
    let key = portal::PublicKey {
        type_id: 3,
        key: key["bytes"].as_str().unwrap().parse().unwrap(),
        fingerprint: key["fingerprint"].as_str().unwrap().parse().unwrap(),
    };
    for _ in 0..2 {
        let error = client
            .owner_public_key_for_session(B256::ZERO, &key, None)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("challenge"), "{error}");
    }
    let challenges = challenges.lock().await;
    assert_eq!(challenges.len(), 2);
    assert_ne!(challenges[0], challenges[1]);
    task.abort();
}

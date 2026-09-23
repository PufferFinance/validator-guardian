//! Fixtures use the public test key 1 and independently generated Foundry hashes.
use alloy::primitives::B256;
use atakit_portal_client::{signing_digest, HashFunction, PortalClient, SignResponse};
use hyper::{service::service_fn, Body, Response};
use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    convert::Infallible,
    sync::{Arc, Mutex},
};
use tokio::net::UnixListener;

struct PortalStub {
    directory: tempfile::TempDir,
    task: tokio::task::JoinHandle<()>,
    requests: Arc<Mutex<Vec<Value>>>,
}

impl PortalStub {
    fn new(responses: Vec<(u16, Value)>) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let listener = UnixListener::bind(directory.path().join("portal.sock")).unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = requests.clone();
        let responses = Arc::new(Mutex::new(VecDeque::from(responses)));
        let task = tokio::spawn(async move {
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                let captured = captured.clone();
                let responses = responses.clone();
                hyper::server::conn::Http::new()
                    .http1_keep_alive(false)
                    .serve_connection(
                        stream,
                        service_fn(move |request| {
                            let captured = captured.clone();
                            let responses = responses.clone();
                            async move {
                                assert_eq!(request.method(), "POST");
                                assert_eq!(request.uri().path(), "/sign-message");
                                assert_eq!(request.headers()["content-type"], "application/json");
                                let body =
                                    hyper::body::to_bytes(request.into_body()).await.unwrap();
                                captured
                                    .lock()
                                    .unwrap()
                                    .push(serde_json::from_slice(&body).unwrap());
                                let (status, body) = responses
                                    .lock()
                                    .unwrap()
                                    .pop_front()
                                    .expect("unexpected extra portal request");
                                Ok::<_, Infallible>(
                                    Response::builder()
                                        .status(status)
                                        .header("content-type", "application/json")
                                        .header("retry-after", "1")
                                        .body(Body::from(body.to_string()))
                                        .unwrap(),
                                )
                            }
                        }),
                    )
                    .await
                    .unwrap();
            }
        });
        Self {
            directory,
            task,
            requests,
        }
    }

    async fn sign_message(
        &self,
        message: &[u8],
        session: Option<B256>,
    ) -> anyhow::Result<SignResponse> {
        let client = PortalClient::new(self.directory.path().join("portal.sock"))
            .with_response_limit(64 * 1024)
            .with_timeout(std::time::Duration::from_secs(15));
        Ok(client
            .sign_message_when_ready(message, HashFunction::Keccak256, session)
            .await?)
    }
}

impl Drop for PortalStub {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn response() -> Value {
    serde_json::from_str(include_str!("fixtures/portal-sign-message.json")).unwrap()
}

#[tokio::test]
async fn accepts_new_portal_response_without_owner_and_uses_domain_hash() {
    let portal = PortalStub::new(vec![(200, response())]);
    let proof = portal.sign_message(b"hello", None).await.unwrap();
    assert_eq!(proof.session_id, B256::repeat_byte(0x11));
    assert_eq!(proof.session_pubkey.type_id, 3);
    assert_eq!(
        proof.signature.to_string(),
        response()["signature"].as_str().unwrap()
    );
    assert_eq!(
        signing_digest(b"hello", HashFunction::Keccak256).to_string(),
        "0xfe5acd658ff3bee68cdbe6ade7688ece13e7744620abb305f6494f1f9a9e8f83"
    );
    assert_eq!(
        *portal.requests.lock().unwrap(),
        vec![json!({
            "message": "0x68656c6c6f", "hash_fn": "keccak256"
        })]
    );
}

#[tokio::test]
async fn retries_initial_readiness_and_preserves_session_pin() {
    let portal = PortalStub::new(vec![
        (
            503,
            json!({"code":"session_not_ready", "state":"initializing_session", "retryable":true}),
        ),
        (200, response()),
    ]);
    portal
        .sign_message(b"hello", Some(B256::repeat_byte(0x11)))
        .await
        .unwrap();
    let requests = portal.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0], requests[1]);
    assert_eq!(
        requests[0]["expected_session_id"],
        format!("0x{}", "11".repeat(32))
    );
}

#[tokio::test]
async fn does_not_retry_session_switch_or_unrelated_failures() {
    for (status, body) in [
        (409, json!({"error":"session_id_mismatch"})),
        (503, json!({"error":"supervisor signing failed"})),
    ] {
        let portal = PortalStub::new(vec![(status, body)]);
        let error = portal.sign_message(b"hello", None).await.unwrap_err();
        assert!(error.to_string().contains(&status.to_string()), "{error}");
        assert_eq!(portal.requests.lock().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn readiness_retries_are_bounded() {
    let portal = PortalStub::new(vec![(503, json!({"code":"session_not_ready"})); 3]);
    assert!(portal.sign_message(b"hello", None).await.is_err());
    assert_eq!(portal.requests.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn rejects_mismatched_hash_identity_signature_and_session_pin() {
    for field in [
        "hash_fn",
        "message_hash",
        "fingerprint",
        "signature",
        "key",
        "type_id",
    ] {
        let mut body = response();
        match field {
            "hash_fn" => body[field] = json!("sha256"),
            "message_hash" => body[field] = json!(format!("0x{}", "00".repeat(32))),
            "signature" => body[field] = json!(format!("0x{}", "00".repeat(65))),
            "fingerprint" => {
                body["session_pubkey"][field] = json!(format!("0x{}", "00".repeat(32)))
            }
            "key" => body["session_pubkey"][field] = json!("0x04"),
            "type_id" => body["session_pubkey"][field] = json!(2),
            _ => unreachable!(),
        }
        let portal = PortalStub::new(vec![(200, body)]);
        assert!(
            portal.sign_message(b"hello", None).await.is_err(),
            "accepted bad {field}"
        );
    }
    let portal = PortalStub::new(vec![(200, response())]);
    assert!(portal
        .sign_message(b"hello", Some(B256::repeat_byte(0x22)))
        .await
        .is_err());
}

#[tokio::test]
async fn accepts_the_same_rotation_proof_as_guardian_module() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/guardian-rotation.json")).unwrap();
    let portal = PortalStub::new(vec![(
        200,
        json!({
            "hash_fn": "keccak256", "message_hash": fixture["message_hash"],
            "signature": fixture["signature"], "session_id": fixture["session_id"],
            "session_pubkey": {"type_id":3, "key":fixture["session_key"], "fingerprint":fixture["session_fingerprint"]}
        }),
    )]);
    let payload = hex::decode(
        fixture["payload"]
            .as_str()
            .unwrap()
            .strip_prefix("0x")
            .unwrap(),
    )
    .unwrap();
    let proof = portal.sign_message(&payload, None).await.unwrap();
    assert_eq!(
        proof.signature.to_string(),
        fixture["signature"].as_str().unwrap()
    );
    assert_eq!(
        proof.session_id.to_string(),
        fixture["session_id"].as_str().unwrap()
    );
}

#[tokio::test]
async fn limits_response_size_even_for_otherwise_valid_proofs() {
    let mut body = response();
    body["padding"] = json!("x".repeat(64 * 1024));
    let portal = PortalStub::new(vec![(200, body)]);
    let error = portal.sign_message(b"hello", None).await.unwrap_err();
    assert!(
        matches!(
            error.downcast_ref::<atakit_portal_client::ClientError>(),
            Some(atakit_portal_client::ClientError::Transport(source))
                if source.to_string().contains("length limit exceeded")
        ),
        "{error}"
    );
}

#[tokio::test]
async fn times_out_when_portal_accepts_but_does_not_respond() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("portal.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let task = tokio::spawn(async move {
        let (_stream, _) = listener.accept().await.unwrap();
        std::future::pending::<()>().await;
    });
    let error = PortalClient::new(path)
        .with_timeout(std::time::Duration::from_secs(15))
        .sign_message_when_ready(b"hello", HashFunction::Keccak256, None)
        .await
        .unwrap_err();
    task.abort();
    assert!(error.to_string().contains("timed out"), "{error}");
}

#[test]
fn attestation_evidence_serializes_a_required_owner_identity() {
    use atakit_portal_client::PublicIdentity;
    use puffersecuresigner::io::remote_attestation::AttestationEvidence;
    let response: SignResponse = serde_json::from_value(response()).unwrap();
    let owner_bundle: atakit_portal_client::EvidenceBundle = serde_json::from_str(include_str!(
        "../crates/atakit-portal-client/tests/evidence-bundle.json"
    ))
    .unwrap();
    owner_bundle.validate_binding([0x42; 32]).unwrap();
    let owner = owner_bundle.recover_owner_public_key().unwrap();
    let evidence = AttestationEvidence {
        session_id: response.session_id.to_string(),
        signature: response.signature.to_string(),
        session_public_key: PublicIdentity {
            type_id: response.session_pubkey.type_id,
            key: response.session_pubkey.key,
        },
        owner_public_key: PublicIdentity {
            type_id: owner.type_id,
            key: owner.key,
        },
    };
    let mut value = serde_json::to_value(&evidence).unwrap();
    assert_eq!(value["owner_public_key"]["typeId"], 3);
    assert_eq!(
        value["owner_public_key"]["key"],
        evidence.owner_public_key.key.to_string()
    );
    assert_eq!(
        serde_json::from_value::<AttestationEvidence>(value.clone()).unwrap(),
        evidence
    );
    value.as_object_mut().unwrap().remove("owner_public_key");
    assert!(serde_json::from_value::<AttestationEvidence>(value).is_err());
}

use super::{BabyCreateRequest, Error, HashFunction, PublicKey, SignResponse, types::*, wire};
use alloy::primitives::B256;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use bytes::Bytes;
use http::{HeaderMap, Method, Request, StatusCode};
use http_body_util::{BodyExt, Full};
use hyper_util::rt::TokioIo;
use rand::RngCore;
use serde::{Serialize, de::DeserializeOwned};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::net::UnixStream;

#[derive(Debug)]
pub struct Reply {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Bytes,
}

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("Failed to generate evidence challenge: {0}")]
    ChallengeGeneration(#[source] rand::Error),
    #[error("Portal returned HTTP {}", .0.status)]
    Http(Box<Reply>),
    #[error("Portal transport failed: {0}")]
    Transport(#[source] Error),
    #[error("Portal request timed out")]
    Timeout,
    #[error("Invalid Portal JSON response: {0}")]
    Decode(#[from] serde_json::Error),
    #[error("Invalid Portal response: {0}")]
    InvalidResponse(String),
    #[error("Invalid Portal signature: {0}")]
    InvalidSignature(String),
}
impl ClientError {
    /// Returns the HTTP error response for inspecting its status, headers, and body.
    ///
    /// Returns `None` for transport, timeout, or local validation errors.
    pub fn http_response(&self) -> Option<&Reply> {
        match self {
            Self::Http(reply) => Some(reply),
            _ => None,
        }
    }
    /// Returns Portal's suggested retry delay as seconds or an HTTP date.
    ///
    /// Returns `None` when unavailable. The caller is responsible for parsing it
    /// and deciding whether to retry.
    pub fn retry_after(&self) -> Option<&str> {
        self.http_response()?
            .headers
            .get("retry-after")?
            .to_str()
            .ok()
    }
}

/// Client for `/run/atakit-portal.sock`, not supervisor RPC or operator HTTPS.
///
/// Each request has its own connection. Only `sign_message_when_ready` retries
/// session-readiness errors; other operations are never automatically retried.
/// Clone cheaply to share configuration between handlers. For large image uploads,
/// use `with_timeout` to allow the transfer and Portal's image import to finish.
#[derive(Debug, Clone)]
pub struct PortalClient {
    socket: PathBuf,
    timeout: Duration,
    response_limit: usize,
}
impl Default for PortalClient {
    /// Creates a client for the workload's Portal socket.
    ///
    /// Uses `$EMULATOR_ROOTFS/run/atakit-portal.sock` when `EMULATOR_ROOTFS` is
    /// nonempty, otherwise `/run/atakit-portal.sock`. Use `new` for an explicit path.
    fn default() -> Self {
        let socket = match std::env::var_os("EMULATOR_ROOTFS").filter(|root| !root.is_empty()) {
            Some(root) => PathBuf::from(root).join("run/atakit-portal.sock"),
            None => PathBuf::from("/run/atakit-portal.sock"),
        };
        Self::new(socket)
    }
}
#[derive(serde::Deserialize)]
struct ReadinessError {
    code: Option<String>,
}

impl PortalClient {
    /// Obtain a validated session signature with a bounded wait for session readiness.
    /// Retries only HTTP 503 `session_not_ready`, with at most three attempts.
    /// The configured timeout covers all requests and delays. Numeric `Retry-After`
    /// is respected with a minimum of one second; missing or invalid values use one second.
    /// Hash and session checks are applied on every successful response.
    pub async fn sign_message_when_ready(
        &self,
        message: &[u8],
        hash_fn: HashFunction,
        expected_session_id: Option<B256>,
    ) -> Result<SignResponse, ClientError> {
        tokio::time::timeout(self.timeout, async {
            for attempt in 0..3 {
                match self
                    .sign_message_with_hash(message, hash_fn, expected_session_id)
                    .await
                {
                    Ok(response) => return Ok(response),
                    Err(error) => {
                        let initializing = match &error {
                            ClientError::Http(reply) if reply.status.as_u16() == 503 => {
                                serde_json::from_slice::<ReadinessError>(&reply.body)
                                    .ok()
                                    .and_then(|body| body.code)
                                    .as_deref()
                                    == Some("session_not_ready")
                            }
                            _ => false,
                        };
                        if !initializing || attempt == 2 {
                            return Err(error);
                        }
                        let delay = error
                            .retry_after()
                            .and_then(|value| value.parse::<u64>().ok())
                            .unwrap_or(1)
                            .max(1);
                        tokio::time::sleep(Duration::from_secs(delay)).await;
                    }
                }
            }
            unreachable!("the final attempt returns a response or error")
        })
        .await
        .map_err(|_| ClientError::Timeout)?
    }

    /// Creates a client for the given Unix socket path.
    ///
    /// Defaults to a 30-second timeout and an 8 MiB response body limit.
    /// The connection is established when a request is made.
    pub fn new(socket: impl Into<PathBuf>) -> Self {
        Self {
            socket: socket.into(),
            timeout: Duration::from_secs(30),
            response_limit: 8 * 1024 * 1024,
        }
    }
    /// Returns the configured Unix socket path.
    pub fn socket(&self) -> &Path {
        &self.socket
    }
    /// Sets the timeout for connecting, sending a request, and reading its response.
    ///
    /// Increase this for operations such as large image uploads.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
    /// Sets the maximum response body size in bytes, including error responses.
    ///
    /// Increase this when requesting large logs. Does not limit upload size.
    pub fn with_response_limit(mut self, bytes: usize) -> Self {
        self.response_limit = bytes;
        self
    }

    /// Sends a custom request when a typed method does not cover the required operation.
    ///
    /// `path` includes any query parameters; `body` contains the raw request bytes.
    /// Returns the original status, headers, and body without decoding them.
    /// Non-2xx responses are returned as `Ok`; check the status before using the body.
    pub async fn request_raw(
        &self,
        method: Method,
        path: &str,
        content_type: &str,
        body: Bytes,
    ) -> Result<Reply, ClientError> {
        tokio::time::timeout(self.timeout, async {
            // Resolve ROOTFS symlinks before the OS applies its Unix socket path limit.
            let socket = tokio::fs::canonicalize(&self.socket).await?;
            let stream = UnixStream::connect(socket).await?;
            let (mut sender, conn) =
                hyper::client::conn::http1::handshake(TokioIo::new(stream)).await?;
            let connection = tokio::spawn(async move {
                let _ = conn.await;
            });
            // Abort the connection even if a timeout/cancellation drops this future.
            struct ConnectionGuard(tokio::task::JoinHandle<()>);
            impl Drop for ConnectionGuard {
                /// Aborts the background HTTP task when the request completes or is cancelled.
                fn drop(&mut self) {
                    self.0.abort();
                }
            }
            let _guard = ConnectionGuard(connection);
            let request = Request::builder()
                .method(method)
                .uri(path)
                .header("Host", "localhost")
                .header("Content-Type", content_type)
                .body(Full::new(body))?;
            let response = sender.send_request(request).await?;
            let (parts, body) = response.into_parts();
            let body = http_body_util::Limited::new(body, self.response_limit)
                .collect()
                .await?
                .to_bytes();
            Ok::<_, Error>(Reply {
                status: parts.status,
                headers: parts.headers,
                body,
            })
        })
        .await
        .map_err(|_| ClientError::Timeout)?
        .map_err(ClientError::Transport)
    }
    /// Sends a request and converts non-2xx responses to `ClientError::Http`.
    ///
    /// Preserves error response contents and propagates transport errors unchanged.
    /// Successful response bodies remain undecoded.
    async fn checked(
        &self,
        method: Method,
        path: &str,
        content_type: &str,
        body: Bytes,
    ) -> Result<Reply, ClientError> {
        let reply = self.request_raw(method, path, content_type, body).await?;
        if !reply.status.is_success() {
            return Err(ClientError::Http(Box::new(reply)));
        }
        Ok(reply)
    }
    /// Sends an empty GET request and decodes the successful JSON response as `T`.
    ///
    /// Propagates HTTP and transport errors; incompatible JSON produces a decode error.
    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, ClientError> {
        let reply = self
            .checked(Method::GET, path, "application/json", Bytes::new())
            .await?;
        Ok(serde_json::from_slice(&reply.body)?)
    }
    /// Serializes `body` as JSON, sends a POST, and decodes the response as `T`.
    ///
    /// Propagates serialization, decoding, HTTP, and transport errors without retrying.
    async fn post<T: DeserializeOwned>(
        &self,
        path: &str,
        body: &impl Serialize,
    ) -> Result<T, ClientError> {
        let reply = self
            .checked(
                Method::POST,
                path,
                "application/json",
                serde_json::to_vec(body)?.into(),
            )
            .await?;
        Ok(serde_json::from_slice(&reply.body)?)
    }
    /// Sends a JSON POST and checks for a 2xx status without decoding the response.
    ///
    /// Used for operations such as stopping or removing resources. Propagates
    /// serialization, HTTP, and transport errors without retrying.
    async fn post_empty(&self, path: &str, body: &impl Serialize) -> Result<(), ClientError> {
        self.checked(
            Method::POST,
            path,
            "application/json",
            serde_json::to_vec(body)?.into(),
        )
        .await?;
        Ok(())
    }
    /// Returns raw status information for forwarding through an application's HTTP API.
    ///
    /// Preserves response headers and body, including non-2xx responses.
    pub async fn status_response(&self) -> Result<Reply, ClientError> {
        self.request_raw(
            Method::GET,
            "/portal-external-api/status",
            "application/json",
            Bytes::new(),
        )
        .await
    }

    /// Returns the current workload and session status.
    ///
    /// Use this to inspect readiness; it does not wait until the session is ready.
    pub async fn status(&self) -> Result<PortalStatus, ClientError> {
        self.get("/portal-external-api/status").await
    }
    /// Signs the raw message bytes with the current session key.
    ///
    /// Checks the returned message hash, key fingerprint, and signature locally.
    /// The signature uses the `ATAKIT_SESSION_SIGN_V1` domain. Successful validation
    /// does not establish TEE trust or confirm that the session is active on-chain.
    pub async fn sign_message(&self, message: &[u8]) -> Result<SignResponse, ClientError> {
        self.sign(message, None, HashFunction::Keccak256).await
    }
    /// Signs a message only with the specified session.
    ///
    /// Use this when a session change must not silently change the signing identity.
    /// Rejects responses from a different session and validates the signature locally.
    pub async fn sign_message_for_session(
        &self,
        message: &[u8],
        session_id: B256,
    ) -> Result<SignResponse, ClientError> {
        self.sign(message, Some(session_id), HashFunction::Keccak256)
            .await
    }
    /// Signs a message with the selected digest algorithm and optional session ID.
    ///
    /// Rejects responses using a different algorithm or session. Validates the
    /// signature locally without checking on-chain registration or TEE trust.
    pub async fn sign_message_with_hash(
        &self,
        message: &[u8],
        hash_fn: HashFunction,
        session_id: Option<B256>,
    ) -> Result<SignResponse, ClientError> {
        self.sign(message, session_id, hash_fn).await
    }

    /// Builds a hex-encoded signing request and validates the response and session ID.
    ///
    /// `expected = None` selects the current session. Returns only responses that
    /// pass local message, key, and signature consistency checks.
    async fn sign(
        &self,
        message: &[u8],
        expected: Option<B256>,
        hash_fn: HashFunction,
    ) -> Result<SignResponse, ClientError> {
        let request = wire::SignMessageRequest {
            message: alloy::hex::encode_prefixed(message),
            hash_fn: Some(hash_fn.as_str().to_owned()),
            expected_session_id: expected.map(|id| id.0),
        };
        let response: wire::SignMessageResponse = self.post("/sign-message", &request).await?;
        let signed = SignResponse::try_from(response)
            .map_err(|e| ClientError::InvalidResponse(e.to_string()))?;
        if signed.hash_fn != hash_fn.as_str() {
            return Err(ClientError::InvalidSignature(
                "unexpected signing hash function".into(),
            ));
        }
        signed
            .validate(message)
            .map_err(|e| ClientError::InvalidSignature(e.to_string()))?;
        if expected.is_some_and(|id| id != signed.session_id) {
            return Err(ClientError::InvalidSignature(
                "session_id does not match the requested session".into(),
            ));
        }
        Ok(signed)
    }
    /// Returns session evidence bound to a fresh caller-generated 32-byte challenge.
    ///
    /// Checks the challenge and the evidence's binding signature. Generate a new
    /// random challenge for every request. Hardware evidence and on-chain session
    /// status still require independent verification.
    pub async fn evidence_bundle(
        &self,
        challenge: [u8; 32],
    ) -> Result<EvidenceBundle, ClientError> {
        let path = format!(
            "/portal-external-api/session/evidence-bundle?challenge={}",
            URL_SAFE_NO_PAD.encode(challenge)
        );
        let bundle: EvidenceBundle = self.get(&path).await?;
        bundle.validate_binding(challenge)?;
        Ok(bundle)
    }
    /// Returns the owner public key matching a trusted `expected_owner` fingerprint.
    ///
    /// Supply a fresh random 32-byte challenge and an independently trusted fingerprint.
    /// No session ID or RPC is needed. Requires owner authorization in the evidence;
    /// returns a 65-byte uncompressed ES256K public key, never the private key.
    /// Does not perform full TEE or on-chain verification.
    pub async fn owner_public_key(
        &self,
        expected_owner: B256,
        challenge: [u8; 32],
    ) -> Result<PublicKey, ClientError> {
        self.evidence_bundle(challenge)
            .await?
            .owner_public_key(expected_owner)
    }
    /// Recovers the owner public key for a previously validated signing response.
    ///
    /// Supply its session ID/key. `None` generates a fresh challenge using OS
    /// randomness; `Some` supplies your own fresh 32-byte challenge. Rejects evidence
    /// from another session or key, including rotation between requests. The
    /// contract must still check this owner's fingerprint and authorization.
    pub async fn owner_public_key_for_session(
        &self,
        session_id: B256,
        session_key: &PublicKey,
        challenge: Option<[u8; 32]>,
    ) -> Result<PublicKey, ClientError> {
        let challenge = match challenge {
            Some(challenge) => challenge,
            None => {
                let mut challenge = [0u8; 32];
                rand::rngs::OsRng
                    .try_fill_bytes(&mut challenge)
                    .map_err(ClientError::ChallengeGeneration)?;
                challenge
            }
        };
        let bundle = self.evidence_bundle(challenge).await?;
        let claims = bundle.claims()?;
        let evidence_session = claims
            .session_id
            .ok_or_else(|| ClientError::InvalidResponse("evidence has no session ID".into()))?
            .parse::<B256>()
            .map_err(|e| ClientError::InvalidResponse(e.to_string()))?;
        let key = PublicKey {
            type_id: claims.session_key.type_id,
            key: claims
                .session_key
                .bytes
                .parse()
                .map_err(|e: alloy::hex::FromHexError| {
                    ClientError::InvalidResponse(e.to_string())
                })?,
            fingerprint: claims
                .session_key
                .fingerprint
                .parse::<B256>()
                .map_err(|e| ClientError::InvalidResponse(e.to_string()))?,
        };
        if evidence_session != session_id
            || key.type_id != session_key.type_id
            || key.key != session_key.key
            || key.fingerprint != session_key.fingerprint
        {
            return Err(ClientError::InvalidResponse(
                "owner evidence does not match the signing session".into(),
            ));
        }
        bundle.recover_owner_public_key()
    }

    /// Uploads a Docker/OCI image tar to the specified baby-container slot.
    ///
    /// Pass the complete archive as raw bytes. Returns the uploaded image ID for
    /// container creation. Large uploads may need a longer timeout.
    pub async fn upload_baby_image(
        &self,
        slot: &str,
        tar: Bytes,
    ) -> Result<BabyImageUpload, ClientError> {
        let path = format!("/baby-container/image/upload?slot={}", encode_query(slot));
        let reply = self
            .checked(Method::POST, &path, "application/octet-stream", tar)
            .await?;
        Ok(serde_json::from_slice(&reply.body)?)
    }
    /// Lists images available to the application, optionally filtered by slot.
    ///
    /// Pass `None` to list all visible slots.
    pub async fn list_baby_images(&self, slot: Option<&str>) -> Result<BabyImages, ClientError> {
        let path = optional_slot("/baby-container/image/list", slot);
        self.get(&path).await
    }
    /// Removes an uploaded image by ID.
    ///
    /// Provide `slot` when the target slot needs to be specified explicitly.
    /// Portal may reject removal if the image is still in use.
    pub async fn remove_baby_image(
        &self,
        image_id: &str,
        slot: Option<&str>,
    ) -> Result<(), ClientError> {
        let body = wire::BabyImageRemoveRequest {
            image_id: image_id.to_owned(),
            slot: slot.map(str::to_owned),
        };
        self.post_empty("/baby-container/image/remove", &body).await
    }
    /// Creates a baby container and returns its instance information.
    ///
    /// The request selects a slot, optional image and instance IDs, and capabilities.
    /// Allowed settings depend on the workload's policy. Success does not mean the
    /// application inside the container has passed a health check.
    pub async fn create_baby_container(
        &self,
        request: &BabyCreateRequest,
    ) -> Result<BabyCreated, ClientError> {
        self.post("/baby-container/create", request).await
    }
    /// Stops the specified baby-container instance without requesting its removal.
    pub async fn stop_baby_container(&self, instance_id: &str) -> Result<(), ClientError> {
        self.post_empty(
            "/baby-container/stop",
            &wire::BabyInstanceRequest {
                instance_id: instance_id.to_owned(),
            },
        )
        .await
    }
    /// Removes the specified baby-container instance.
    ///
    /// Portal may require the instance to be stopped first.
    pub async fn remove_baby_container(&self, instance_id: &str) -> Result<(), ClientError> {
        self.post_empty(
            "/baby-container/remove",
            &wire::BabyInstanceRequest {
                instance_id: instance_id.to_owned(),
            },
        )
        .await
    }
    /// Returns the application's visible baby-container instances and images.
    pub async fn list_baby_containers(&self) -> Result<BabyContainers, ClientError> {
        self.get("/baby-container/list").await
    }
    /// Returns a log snapshot for the specified baby-container instance.
    ///
    /// `max_bytes` requests a byte limit; `None` uses Portal's default.
    /// This is not a live stream, and the client's response size limit still applies.
    pub async fn baby_container_logs(
        &self,
        instance_id: &str,
        max_bytes: Option<usize>,
    ) -> Result<BabyLogs, ClientError> {
        let mut path = format!(
            "/baby-container/logs?instance_id={}",
            encode_query(instance_id)
        );
        if let Some(limit) = max_bytes {
            path.push_str(&format!("&max_bytes={limit}"));
        }
        self.get(&path).await
    }
}
/// Appends an encoded slot query parameter to a path with no existing query.
fn optional_slot(path: &str, slot: Option<&str>) -> String {
    match slot {
        Some(slot) => format!("{path}?slot={}", encode_query(slot)),
        None => path.into(),
    }
}
/// Percent-encodes UTF-8 bytes, preserving only URI unreserved characters.
fn encode_query(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            encoded.push(byte as char);
        } else {
            use std::fmt::Write;
            write!(encoded, "%{byte:02X}").unwrap();
        }
    }
    encoded
}

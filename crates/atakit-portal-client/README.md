# atakit-portal-client

Rust client for atakit Portal's workload Unix socket, defaulting to
`/run/atakit-portal.sock`. Independent of the secure-signer application and its
HTTP server, business contract, and startup environment. Unix platforms only.

`PortalClient::default()` uses `$EMULATOR_ROOTFS/run/atakit-portal.sock` when
`EMULATOR_ROOTFS` is nonempty, so the same application works in the native
emulator and in a CVM or container. Without that variable, it uses
`/run/atakit-portal.sock`. `PortalClient::new(path)` always uses the explicit path,
regardless of environment variables. Neither constructor reads `PORTAL_SOCKET`.

This package is currently consumed from source; it has not been published to
crates.io. Other applications can use:

```toml
[dependencies]
atakit-portal-client = { path = "../atakit_examples/crates/atakit-portal-client" }
```

```rust,no_run
use atakit_portal_client::PortalClient;

# async fn example() -> Result<(), atakit_portal_client::Error> {
let portal = PortalClient::default();
let status = portal.status().await?;
let signed = portal.sign_message(b"hello").await?;
# Ok(())
# }
```

Supports status, message signing (with optional expected session), evidence
bundles, owner public-key recovery, and baby-container image/container operations.
Transport timeout and maximum response size are configurable. HTTP errors retain
status, body, and Retry-After. Ordinary requests are never automatically retried.

`sign_message` validates the returned signature locally. `evidence_bundle`
requires a fresh caller-generated 32-byte challenge and validates the response
binding. `owner_public_key(expected_owner, challenge)` additionally checks a
caller-trusted fingerprint. `EvidenceBundle::recover_owner_public_key` only checks
the advertised owner's key consistency; validate the evidence binding first.
These operations do not establish TEE trust or query the session registry.

From the workspace root:

```sh
cargo test -p atakit-portal-client
cargo check -p atakit-portal-client --lib
```

The ignored live test requires `EMULATOR_SOCKET` and
`EMULATOR_OWNER_FINGERPRINT`. Axum is used only by the test mock server, not by
the client library.

Status fields and requests follow the versioned Portal protocol documented in
[PROTOCOL.md](PROTOCOL.md). Required fields remain required; unknown fields are
ignored rather than exposed as extension maps. Portal's evidence response uses `serde_json::Value`; all fields participate
in JCS-canonicalized binding verification. Use `bundle.claims()?` to read typed binding, session
key, and owner fields without altering that document.

Choose Portal's other supported signing algorithm with
`sign_message_with_hash(message, HashFunction::Sha256, expected_session_id)`.
The default signing methods use Keccak256. Raw string-based protocol types live
in `wire`; the signing methods return parsed values for local validation and
contract calls.

### Waiting for session readiness

`sign_message_when_ready(message, hash_fn, expected_session_id)` retries only
HTTP 503 responses with `code = "session_not_ready"`, up to three attempts.
`with_timeout` bounds the whole operation, including retry delays. Numeric
`Retry-After` values are honored with a one-second minimum; other values use one
second. Session mismatch, transport errors, and other HTTP failures are returned
immediately. Existing signing methods remain single-attempt operations.

`signing_digest(message, hash_fn)` computes the domain-separated Keccak256 or
SHA-256 digest for contract verification. Supply raw message bytes, not a digest
or a message already prefixed with the Portal signing domain.

`owner_public_key_for_session(session_id, session_key, challenge)` recovers an
owner identity using a fresh OS-generated challenge when passed `None`. Pass
`Some(challenge)` to provide your own fresh 32-byte challenge. It recovers the
owner identity from request-bound evidence and checks it matches a validated
signing response's session ID and public key. It rejects rotation between the
signing and evidence requests. The returned identity is for constructing proofs;
on-chain owner/workload authorization still belongs to the verifier.

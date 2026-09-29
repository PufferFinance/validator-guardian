# Protocol source

The client follows **atakit-portal commit
`079cb59c3434926890f0017a19b9684f86dddfb5`**. Update protocol types and fixtures
together when adopting another Portal version.

| Protocol types | Source |
| --- | --- |
| Status, platform, chain, prover | `atakit-portal/crates/atakit-portal-api/src/handlers.rs` |
| Cloud provenance and enums | `atakit-portal/crates/atakit-portal-platform/src/cloud.rs` |
| Signing and baby-container messages | `atakit-portal/crates/atakit-portal-api/src/workload.rs` |
| Evidence response envelope | `atakit-portal/crates/atakit-portal-api/src/session_http.rs` |
| Evidence binding, key, owner and authorization | `atakit/crates/atakit-attestation/src/session.rs`, used by Portal |

Protocol fields retain their required/optional distinctions, enums, and
unknown-field rules. No extension maps are exposed. Returned strings are owned,
including Portal's static build-profile string. Signing session IDs serialize
as canonical lowercase hex and are omitted when unset, matching Portal's custom
request decoder. Other request fields retain their protocol definitions.

`wire` contains the original string-based signing messages and complete nested
evidence identity types. `PublicKey` and `SignResponse` are explicitly parsed
client models for cryptographic operations, converted from those wire types.
They are not alternative protocol definitions. `EvidenceClaims` is a read-only
projection using the complete nested protocol types; the full evidence JSON
remains available for verification.

The evidence envelope uses `serde_json::Value`, exactly as Portal does. Its full
value is JCS-canonicalized before request-binding verification. Both Keccak256
and SHA-256 signing are supported; Keccak256 is Portal's default.

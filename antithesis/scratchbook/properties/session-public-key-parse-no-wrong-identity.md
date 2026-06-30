# Property: parse_session_public_key never silently fabricates a wrong PublicIdentity

slug: session-public-key-parse-fallback
focus: (4) Protocol Contracts — serde/API shape (W9)
priority: MEDIUM (downgraded scope: W9 is LIVENESS-only — verifier fails closed on
typeId/curve mismatch, confirmed; and the in-repo producer emits raw hex so the
JSON branch is dead for SUT-originated payloads)
confidence: HIGH (behavior confirmed in code + atakit struct + feat/tdx contract)

## Origin
`src/enclave/guardian/mod.rs::parse_session_public_key` lines **231-258**:
```
if let Ok(identity) = serde_json::from_str::<automata_cvm_agent::PublicIdentity>(session_public_key) {
    return Ok(PublicIdentity { typeId: identity.type_id, key: identity.key });
}
// Fall back to hex-encoded secp256k1 key
let key_bytes = hex::decode(strip_0x_prefix!(session_public_key))?;
Ok(PublicIdentity { typeId: 3, key: key_bytes.into() })  // <-- hardcoded ES256K, swallows JSON error
```
- The JSON branch uses `automata_cvm_agent::PublicIdentity` =
  `automata_tee_workload_measurement::stubs::PublicIdentity`
  (`{ type_id: u8, key: Bytes }`, `#[serde(rename_all="camelCase")]` →
  expects `{"typeId":N,"key":"0x.."}`). `AlgoId::Es256K = 3`.
- **W9: the JSON parse error is swallowed.** If the field is *meant* to be JSON
  but is malformed (e.g. `typeId` present but key truncated, or wrong field
  names), `from_str` fails, control falls through, and the function hex-decodes
  the WHOLE string and stamps `typeId: 3` unconditionally. Only hex-decode failure
  surfaces an error; a typeId other than 3 in a malformed-but-hex string is lost.

## External verifier it must match
The `PublicIdentity { typeId, key }` is passed to
`SessionRegistry.verifySessionSignature(sessionId, sessionKey, message, sig)`
(session_registry.rs:51-76, called from mod.rs:184-192). On-chain, the verifier
dispatches on `typeId` to pick the signature scheme (ES256K=3 secp256k1 vs
ES256=2 P-256 vs RS256=1). **[CONFIRMED fail-closed — see Investigation Log Q2]**
`SignatureVerifier.verify` reverts `UnsupportedAlgorithm(typeId)` on any unknown
typeId and the ES256K path requires a 65-byte `0x04`-prefixed key; plus
`verifySessionSignature` first checks the key fingerprint (which binds typeId).
A fabricated typeId=3 on a key that is actually a different algo → fingerprint
mismatch or recovered-address mismatch → returns false (fail-closed). It does NOT
mis-verify.

## What breaks
- A genuinely secp256k1-hex session key → fine (the common case).
- A session key that SHOULD have been JSON with typeId != 3 but failed to parse →
  silently coerced to typeId 3 → wrong-scheme verification → `bail!`/500 (custody
  refused) — fail-closed, a liveness/UX problem, hard to diagnose because the root
  cause (a JSON parse error) was swallowed.
- Subtle correctness risk: the fallback applies `strip_0x_prefix` and hex-decodes
  the *entire* string including any JSON braces → garbage key bytes; verification
  fails closed.

## Scope caveat
Same gating as the reconstruction property — only reached when
`verify_session=true` (currently false in prod). Document as inert-today.

## Property statement
- `Always`: when `parse_session_public_key` returns Ok with `typeId == 3` via the
  fallback, the input was valid hex AND not a (different-typeId) JSON object —
  i.e. the fallback must not mask a JSON shape it should have honored.
- `Sometimes(reachable)`: the JSON branch is taken (proves real
  `{"typeId":3,"key":...}` inputs are parsed structurally, not via fallback).
- Weaker but cheap: `Always(returned typeId is one the SessionRegistry verifier
  supports)`.

## Antithesis angle
Pure input-space coverage: feed JSON, hex, 0x-hex, malformed-JSON-that-is-also-hex,
JSON with typeId 2/3/other. Assert the fallback never silently changes the
algorithm id of a parseable identity. Not fault-driven; this is fuzzing the API
shape boundary. Antithesis-guided random strings exercise the swallowed-error edge
much better than hand-written tests (of which there are zero for this fn).

## Instrumentation status: MISSING
No tests cover `parse_session_public_key`. Add an `assert_always` after the
fallback that the input did not also parse as a typeId!=3 JSON identity.

## Open questions
- ~~Does the real SessionRegistry verifier reject a typeId/key-curve mismatch
  (fail-closed)?~~ **RESOLVED (code). Fail-closed confirmed.**
  `SignatureVerifier.verify` (workload-measurement submodule,
  `SignatureVerifier.sol:31-47`) dispatches on `typeId`: RS256→RSA, ES256→P256,
  ES256K(3)→secp256k1, **else `revert UnsupportedAlgorithm(typeId)`** — never a
  silent wrong-curve fallthrough. The ES256K path (`_verifySecp256k1`,
  :113-132) additionally requires a 65-byte `0x04`-prefixed key (else returns
  false) and recovers via `ECDSA.tryRecoverCalldata`, comparing against the
  address derived from the supplied key. A coerced `typeId=3` over a key that is
  NOT a valid secp256k1 uncompressed pubkey fails closed (returns false or the
  recovered address mismatches). Before dispatch, `verifySessionSignature`
  (`SessionRegistry.sol:411-427`) checks `LibKey.computeKeyFingerprint(sessionKey)
  == stored sessionKeyFingerprint`, where the fingerprint =
  `keccak256(abi.encode(KEY_DOMAIN, typeId, key))` — so the typeId is bound into
  the fingerprint and a wrong typeId fails the fingerprint check first. ⇒ **W9 is
  LIVENESS-only, not a correctness hole.** A swallowed-JSON / coerced-typeId can
  only cause a fail-closed reject (custody refused), never a false verify.
- ~~What format does the CVM agent emit for `session_public_key` — JSON or raw
  hex?~~ **RESOLVED (code) for the in-repo producer: raw 0x-hex, NOT JSON.** The
  SUT's own validator producer sets
  `session_public_key: evidence.session_public_key.key.to_string()`
  (`src/enclave/validator/mod.rs:128`) — i.e. only the **key bytes** of the
  `PublicIdentity`, rendered by `alloy::Bytes`' Display as a `0x`-prefixed hex
  string (NO `typeId`, NO JSON braces). So in this system's keygen path,
  `parse_session_public_key` takes the **hex fallback** branch (stamping
  `typeId:3`), and the JSON branch is effectively dead for SUT-originated payloads.
  This makes the W9 swallowed-error concern even narrower: the JSON branch would
  only ever fire for an externally-injected JSON string. (partial: a non-SUT
  producer — e.g. a future atakit/reef change that forwards the full JSON
  `PublicIdentity` — could exercise the JSON branch; that is a version-compat
  contingency, not the current behavior.)

### Investigation Log

#### Q1 (signing-format overlap): does the agent emit a key the verifier can use raw?
- **Examined:** the producer's emission of `session_public_key`
  (`validator/mod.rs:128`); atakit `PublicIdentity::secp256k1` (stubs.rs:60-70);
  `alloy::Bytes` Display impl (`0x`-prefixed hex).
- **Found:** Producer emits only the 65-byte uncompressed key as `0x`-hex. atakit
  always builds the session/owner identity via `PublicIdentity::secp256k1`
  (type_id=3, uncompressed 65B), consistent with what the guardian's hex fallback
  stamps (`typeId:3`).
- **Conclusion:** RESOLVED for the in-repo path; the hex fallback's hardcoded
  typeId=3 is correct for every key atakit currently produces.

#### Q2: typeId/curve match + fail-closed on mismatch
- **Examined:** puffer-contracts `feat/tdx` workload-measurement submodule —
  `types/Common.sol` PublicIdentity, `types/Constants.sol` ALGO_ID_*,
  `SessionRegistry.sol::verifySessionSignature`, `SignatureVerifier.sol::verify` +
  `_verifySecp256k1`, `LibKey` fingerprint. Rust/atakit `AlgoId`/`PublicIdentity`.
- **Found:** `PublicIdentity { uint8 typeId; bytes key; }` (Common.sol:33-37) ==
  Rust shape. `ALGO_ID_ES256K = 3` (Constants.sol:52) == `AlgoId::Es256K = 3` ==
  guardian's hardcoded `typeId:3`. verify() reverts `UnsupportedAlgorithm` on
  unknown typeId (FAIL CLOSED); secp256k1 path requires 65-byte `0x04`-prefixed
  key; fingerprint binds typeId so a mismatch is rejected pre-dispatch.
- **Conclusion:** RESOLVED. typeId=3/ES256K/secp256k1/65-byte all agree; the
  registry fails closed on a curve/typeId mismatch. W9 downgraded to liveness-only.

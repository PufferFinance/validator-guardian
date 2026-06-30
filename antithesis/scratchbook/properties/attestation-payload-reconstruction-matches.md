# Property: reconstructed validator-remote-attestation payload == the payload the CVM agent signed

slug: attestation-payload-reconstruction-matches
focus: (4) Protocol Contracts — manual raw-byte keccak builder (verify path)
priority: HIGH (gated behind verify_session, which is OFF in prod today — see note)
confidence: HIGH (builder confirmed; CORRECTION — the validator-side producer IS
in this repo and uses the SAME shared builder, so field order / threshold width /
pad match by construction; only the closed-source DCAP signer is out-of-repo)

## Origin / where the bytes are built
- Builder: `src/enclave/shared/mod.rs::build_validator_remote_attestation_payload`
  lines **179-225**. Manual `sha3::Keccak256` byte stream (NOT abi.encode):
  1. `validator_pk_set.to_bytes()`            (blsPubKeySet)
  2. `validator_pk_set.public_key().to_bytes()` (blsPubKey, 48B)
  3. `signature.to_vec()`                      (BLS sig, 96B)
  4. `deposit_data_root` (32B)
  5. for each i in `enc_sk_shares.zip(guardian_pks)`:
     - `hex::decode(sk_share)`   (blsEncPrivKeyShares[i])
     - `validator_pk_set.public_key_share(i).to_bytes()`  (blsPubKeyShares[i])
     - `g_pk.serialize()`        (guardianPubKeys[i], uncompressed 65B)
  6. `validator_pk_set.threshold().to_be_bytes()`  (threshold)
  then `digest = keccak256(...)`; result = **digest (32B) || 32 zero bytes** = 64B
  (lines 214-224).
- Consumer: `verify_session_evidence` (mod.rs:115-225) calls this at **137-147**,
  then `message = keccak256(payload)` (line 150) and passes `message` to
  `SessionRegistry.verifySessionSignature(session_id, session_key, message, sig)`.
  i.e. the on-chain verifier checks the CVM agent signed
  `keccak256( digest32 || zero32 )`.

## External verifier it must byte-match
The Automata CVM agent signs this payload **at validator-enclave keygen time**.
**[CORRECTED — see Investigation Log Q4]** The producing side IS in this repo:
`src/enclave/validator/mod.rs::attest_fresh_bls_key` builds the byte stream via the
**same** `build_validator_remote_attestation_payload` and calls
`AttestationEvidence::new(&payload)` to have the agent sign it; the guardian
RECONSTRUCTS the identical bytes (same function) to re-derive the signed message
for on-chain `verifySessionSignature`. So byte-layout agreement is structural; the
only out-of-repo element is the closed-source DCAP agent's signing primitive
(confirmed raw keccak256 in every in-repo signer — Q1). The property is: guardian's
reconstruction == the exact byte stream the agent signed at keygen.

## Hazards baked into the builder (drift sources)
- **F2 — `.zip()` silent truncation (mod.rs:200).** `enc_sk_shares` and
  `guardian_pks` are zipped; if `len(enc_sk_shares) != len(guardian_pks)`, the
  longer is silently truncated and trailing shares are DROPPED from the hash. A
  length desync between the two caller-supplied vectors produces a *different*
  digest → verification fails closed (breaks provisioning liveness) OR, with a
  crafted pair, hashes a subset the signer never intended. Note also the index
  `i` from `enumerate()` is used to pull `public_key_share(i)` — if the shares are
  reordered relative to keygen, `public_key_share(i)` no longer corresponds and
  the digest diverges.
- **W6 — `threshold().to_be_bytes()` on a platform-dependent `usize`
  (mod.rs:212).** `PublicKeySet::threshold()` returns `usize`
  (blsttc lib.rs:707). On a 64-bit build `to_be_bytes()` = 8 bytes; on a 32-bit
  build = 4 bytes. The off-line signer and the guardian MUST agree on width or the
  digest differs. A cross-arch build (or the validator-side using a fixed-width
  encoding) silently breaks verification. Strong `Always` candidate: "threshold
  serialization is exactly 8 bytes / matches the agreed width."
- **The 32 zero-byte pad (lines 216-222)** — fixed 64-byte layout. Both sides must
  pad identically; if the signer hashes only the 32-byte digest (no pad) the
  message mismatches.

## What breaks on drift
- Any byte mismatch → `verifySessionSignature` returns false → `verify_session_evidence`
  `bail!`s → 500 → custody refused → registration stalls (liveness). No fail-open
  risk here (fails closed), but a *crafted* truncation/reorder could let an
  attacker-shaped payload still hash to something the agent signed for a different
  share set (correctness concern, lower likelihood).

## IMPORTANT scope caveat
This entire path is gated on `request.verify_session` (mod.rs:71), and reef sends
`verify_session: false` with empty session fields (new_registration.rs:257-266).
**In production today this builder is never exercised.** So the property is:
(a) high value IF/WHEN attestation is turned on, (b) currently dead-code-ish.
Flag clearly to the property-evaluation pass: this protects a path that is
presently inert but is the documented security story (G1/G3).

## Property statement
For generated valid `(pk_set, signature, dd_root, enc_sk_shares, guardian_pks)`
of EQUAL length, the reconstructed 64-byte payload (and `keccak256` of it) MUST
equal the reference payload the off-line agent signed; and for UNEQUAL-length
inputs the builder must either reject or the mismatch must be detectable
(currently it silently truncates — assert this is `Unreachable` in well-formed
operation, or `Sometimes` to surface that truncation can happen).

## Antithesis angle
- Differential/oracle property with diverse share counts & threshold values
  (input-space coverage). `Always(reconstructed == reference)` when lengths match.
- `Sometimes(len(enc_sk_shares) != len(guardian_pks))` to prove the truncation
  branch is reachable under fuzzed input → then `Always(no silent truncation
  affects the signed digest)` (i.e. assert lengths equal before hashing).
- `Always(threshold serialization width == 8)` to pin W6.
- Genuinely fuzz-driven (varied counts/threshold/lengths), less fault-injection.

## Instrumentation status: MISSING
No reference signer available in-repo; SUT-side instrumentation needs either a
test-vector captured from the validator/atakit keygen, or a re-implementation of
the same byte builder to differential-test against. Add a length-equality
`assert_always` and a width `assert_always` cheaply even without the reference.

## Open questions
- ~~Where does the validator enclave / atakit build the signed payload, and does it
  use the SAME field order / `usize` width / pad?~~ **RESOLVED (code) — the
  premise that the builder is "not in this repo" was WRONG.** The validator-side
  PRODUCER is `src/enclave/validator/mod.rs::attest_fresh_bls_key` (:96-108, and
  the test at :195), and it calls the **exact same**
  `crate::enclave::shared::build_validator_remote_attestation_payload` that the
  guardian VERIFIER calls (`guardian/mod.rs:137`). Producer and verifier are the
  same function in the same crate, compiled together → identical field order,
  identical threshold width, identical 64-byte pad BY CONSTRUCTION. The only
  cross-process boundary is the CVM agent signing the bytes it is handed, which is
  raw `keccak256(payload)` (see Q1, atakit `sim/state.rs::sign`). So the
  reconstruction == the bytes the agent signed, provided the validator and guardian
  run builds of the same crate (a version-compat assumption, not a byte-layout one).
- ~~Is the 32-zero-byte pad meaningful, or could the agent sign only 32 bytes?~~
  **RESOLVED (code).** The producer (`attest_fresh_bls_key` → `AttestationEvidence::new(&payload)`)
  hands the agent the full 64-byte padded vec (`shared/mod.rs:216-224`), and the
  agent signs `keccak256` of whatever bytes it receives. Both sides use the same
  builder, so both commit to the same 64-byte input. The pad is not an
  agent-meaningful artifact; it is just part of the shared message both sides hash.
- **(partial)** Are `enc_sk_shares` and `guardian_pks` guaranteed equal-length?
  On the PRODUCER side they are derived from the SAME `recipient_keys` vector
  (`validator/mod.rs:100-107`) so they are equal-length by construction. On the
  VERIFIER side they come from two separate `BlsKeygenPayload` fields
  (`bls_enc_priv_key_shares` and `guardian_eth_pub_keys`) carried across the reef
  boundary — F2 silent-`.zip()`-truncation is still live IF those two fields
  desync in transport/BFF. (Caller-guarantee = needs reef/BFF confirmation, but the
  in-process producer never creates a mismatch.)

### Investigation Log

#### Q1: Does the CVM agent sign `keccak256(payload)` raw (no EIP-191)?
- **Examined:** Same as the rotate-key file. Verifier consumes at
  `guardian/mod.rs:150` `let message = keccak256(&payload)`; producer signs at
  `validator/mod.rs:112` `AttestationEvidence::new(&payload)`.
- **Found:** atakit `sim/state.rs::sign()` = raw `keccak256(message)` + `sign_hash`,
  no prefix. Both atakit commits identical. No EIP-191 anywhere.
- **Conclusion:** RESOLVED (code path). The guardian's `keccak256(payload)`
  reconstruction-and-verify matches what the agent signs. Residual = closed-source
  production DCAP agent (human-input).

#### Q4: validator/atakit payload builder — field order / `usize` width / pad
- **Examined:** `grep` for `build_validator_remote_attestation_payload` and
  `threshold().to_be_bytes` / `public_key_share(i)` across `src/`;
  `src/enclave/validator/mod.rs` (producer); `rust-toolchain.toml` (channel 1.91);
  `container/Dockerfile` (`FROM rust:1.91-bookworm`, no `--target` override →
  native `x86_64-unknown-linux-gnu`); blsttc `PublicKeySet::threshold(&self) ->
  usize` (lib.rs:707); `src/constants.rs` (`ETH_UNCOMPRESSED_PK_BYTES = 65`);
  `eth_keys.rs::eth_pk_*_uncompressed` (65B).
- **Found:** Producer (`attest_fresh_bls_key`) and verifier (`verify_session_evidence`)
  both call the single shared builder `shared/mod.rs:179`. Same field order, same
  `threshold().to_be_bytes()`, same 64-byte pad — they are the same code. Target
  build is x86_64 → `usize` is 8 bytes → `to_be_bytes()` = **8 bytes** on both
  sides. Guardian pubkey field is 65-byte uncompressed (`g_pk.serialize()`) on both
  sides. NB: on the producer, `enc_sk_shares` and `guardian_pks` are both projected
  from one `recipient_keys` vec, so equal-length by construction.
- **Not found:** Any *separate* atakit/validator-side byte builder that could
  diverge in field order or threshold width — there is none; it is shared.
- **Conclusion:** RESOLVED (code). Field order / threshold width (8 bytes on the
  x86_64 target) / 64-byte pad all match because producer and verifier share one
  function. This is a STRONGER result than the file originally assumed. Property
  largely CONFIRMED in-process; the remaining hazards are (a) cross-build version
  skew between validator and guardian binaries (focus-10 dependency-drift, not a
  byte-layout bug), and (b) F2 `.zip()` truncation on the verifier if the two
  reef-carried length fields desync. The `Always(threshold serialization == 8
  bytes)` assertion remains valid and cheap.

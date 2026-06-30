# W7-3 — Compressed/uncompressed ETH-pubkey identity split: the value the client receives is NOT the on-disk identity

## Origin / intuition
keygen hands the caller the **uncompressed** (65-byte) enclave ETH pubkey, but
the key is stored on disk — and looked up at custody time — under the
**compressed** (33-byte) form. The two encodings denote the same curve point,
so the round-trip works *only if every hop re-parses the point and the guardian
re-compresses it consistently*. The "identity" of the guardian enclave key is
thus represented two different ways at two different boundaries, and the list
endpoint exposes only the compressed form — so the pubkey a client holds (from
keygen) can never be `==`-matched against the pubkey the list endpoint returns.
This is a latent confused-identity / "key exists but is unrecognizable" footgun
that no lens audits because, on the happy path, the encodings happen to reconcile.

## Files / functions / line numbers
- **Returned uncompressed** — `src/enclave/types.rs:18-24`
  `KeyGenResponse::from_eth_key`: `hex::encode(pk.serialize())` → 65-byte
  uncompressed, prefixed `0x`. Comment literally says `// uncompressed`.
- **Stored compressed** — `src/crypto/eth_keys.rs:87-95` `save_eth_key` →
  `eth_pk_to_hex(&pk)` → `pk.serialize_compressed()` (line 28-30) → 33-byte
  filename.
- **Looked up compressed** — `src/enclave/guardian/mod.rs:64-68`
  `verify_and_sign_custody_received`:
  `fetch_eth_key(&eth_pk_to_hex(&request.guardian_enclave_public_key))` — it
  re-compresses whatever point the caller deserialized into
  `guardian_enclave_public_key` (a `libsecp256k1::PublicKey`). Works *iff* the
  caller faithfully preserved the point through its own (de)serialization.
- **List exposes compressed only** — `bin/guardian.rs:47-49` GET `/eth/v1/keygen`
  → `list_eth_keys` → filenames are the compressed hex.
- **Cross-repo reconciliation hop** —
  `reef/.../new_registration.rs:205-230`: reef takes its configured guardian
  pubkey hex, `hex::decode`s it, and `EthPublicKey::parse_slice(..., None)` —
  `parse_slice` auto-detects compressed vs uncompressed by length, so reef's
  configured key may be in *either* form and still round-trips. The guardian
  then re-compresses. The identity survives only because of this double-parse.
- **Serde wire shape wildcard (compounding)** — `ValidateCustodyRequest`
  serializes `guardian_enclave_public_key: EthPublicKey` via libsecp256k1's
  *human-readable* serde impl, which is **base64**, not hex (SUT §2). So the
  guardian's identity travels the wire as base64, is stored as compressed hex,
  and is returned from keygen as uncompressed hex — **three encodings of one
  identity** across three boundaries.

## What breaks
- **Client-side identity matching is impossible without re-deriving.** A client
  that records the keygen response (uncompressed) and later calls the list
  endpoint (compressed) cannot string-compare to confirm "my key is present."
  Any tooling that pins the guardian by its keygen-returned pubkey string will
  silently treat the same key as a different key. → "key exists but can't be
  found" by string identity.
- **Fragile under any encoding drift.** If a future caller passes the
  guardian_enclave_public_key in a form whose serde/parse path doesn't
  perfectly reconstruct the point (e.g. base64-vs-hex mix-up, an extra/missing
  byte), `eth_pk_to_hex` produces a *different* compressed filename →
  `fetch_eth_key` 404 → custody 500, with the misleading message "Could not
  fetch guardian enclave public key" even though the key is on disk.
- **No canonical-form assertion anywhere.** Nothing checks that the point
  serialized into the request, the point used for the filename, and the point in
  the keygen response are the same curve point.

## Antithesis angle
- **`Always`** (SUT-side, in keygen handler): *"the curve point in the
  uncompressed `pk_hex` returned to the client re-compresses to the exact
  filename just written to disk."* Encode by re-parsing the returned
  uncompressed hex, compressing, and asserting equality with the saved filename.
  Anchors the encoding round-trip.
- **`Always`** (custody handler): *"the compressed lookup key derived from
  `request.guardian_enclave_public_key` denotes the same point the client
  received from keygen for this enclave."* (Requires correlating keygen→custody;
  workload can carry the keygen pubkey forward.)
- **`Reachable`**: *"custody fails with 'Could not fetch guardian enclave public
  key' while the eth_keys dir is non-empty"* — the confused-identity smoking gun
  (distinguish from the genuine empty-dir / no-prior-keygen case).
- **`Sometimes`**: *"a keygen-returned pubkey string is byte-for-byte unequal to
  any string returned by the list endpoint for the same key"* — documents the
  identity-representation split as a real, reachable state (will essentially
  always hold → effectively `Reachable`, but stating it pins the invariant).

## Why the other six lenses miss it
- **Data Integrity / Idempotency** verifies that the *bytes on disk* round-trip;
  it does not compare the *returned* representation against the *stored* one
  because they live at different endpoints.
- **Protocol Contracts** audits ABI preimages and serde shapes for the signed
  payloads, but `guardian_enclave_public_key`'s base64-vs-hex and
  compressed-vs-uncompressed split is an *identity*-encoding issue, not a
  signed-message issue.
- **Security Boundaries** treats the pubkey as an opaque identity (W4); it never
  asks whether the same identity is representable/matchable across the keygen,
  list, and custody surfaces.
- The other lenses' framing assumes "a pubkey is a pubkey"; the novelty is that
  here *one key has three on-the-wire/on-disk faces.*

## Open questions (and why they matter)
- Why does keygen return uncompressed while everything else is compressed —
  intentional (consumer needs uncompressed for some ABI) or an oversight? →
  Decides whether the fix is "return compressed too" or "this split is load-
  bearing and must be asserted, not removed."
- Does any consumer string-match the keygen pubkey against the list endpoint? →
  If yes, the confused-identity bug is live, not theoretical.

## Instrumentation: MISSING
Needs SUT-side assertions correlating the returned hex, the stored filename, and
the request's deserialized point. No SDK present.


---

> **[merged]** consolidated from discovery focus file `prop-focus-4/serde-key-encoding-contract.md`

# Property: enclave-API key/sig encodings round-trip with the reef consumer (base64 vs hex, case)

slug: serde-key-encoding-contract
focus: (4) Protocol Contracts — serde wire-shape; (10) Version Compatibility (reef↔guardian)
priority: MEDIUM
confidence: HIGH (encodings confirmed in both repos)

## Origin / the mixed-encoding boundary
`src/enclave/types.rs` `ValidateCustodyRequest` (lines 83-93,
`#[serde(rename_all="camelCase")]`):
- `guardian_enclave_public_key: ecies::PublicKey` — serializes via libsecp256k1's
  Serialize impl, which is **base64** (human-readable), NOT hex. Everything else
  on the wire is hex.
- `keygen_payload: BlsKeygenPayload` is **snake_case** fields (types.rs:134-147,
  no rename_all) embedded inside a camelCase outer struct → MIXED conventions in
  one JSON body.
- `validator_index: ValidatorIndex`, `chain_id: u64`, `guardian_module_address:
  String`.
`ValidateCustodyResponse` (95-103, camelCase): `enclave_signature`, `bls_pub_key`,
`withdrawal_credentials`, `deposit_signature`, `deposit_data_root` — all hex
strings.

## The consumer it must round-trip with
reef builds the request in
`reef-guardian/.../provision_or_skip/new_registration.rs:246-270`:
- `guardian_enclave_public_key: guardian_enclave_pubkey` (an `EthPublicKey` =
  `ecies::PublicKey`) — relies on the SAME base64 serde impl. If the guardian and
  reef ever pin different `libsecp256k1`/`ecies` versions whose Serialize impl
  differs (hex vs base64, compressed vs uncompressed), the field silently fails to
  deserialize → 422/400, custody never runs.
- All other fields hex-encoded by reef (`hex::encode(...)`) matching the
  guardian's snake_case `BlsKeygenPayload`.
- `guardian_enclave_public_key` is later re-derived inside the guardian via
  `eth_pk_to_hex` (compressed, mod.rs:65) to look up the on-disk key file — the
  base64 wire form vs compressed-hex filename is an internal footgun
  (sut-analysis §2 key affinity), separate from the wire round-trip.

## Version-compatibility hazard (focus 10)
- `libsecp256k1 = "0.7.1"`, `ecies = "=0.2.7"` (pinned exact). The pin protects
  the guardian, but reef must use a serde-compatible version. The
  base64-vs-hex asymmetry is exactly the kind of thing that breaks on a dependency
  bump and is invisible until a request fails to deserialize.
- camelCase outer + snake_case inner means a future "tidy up to all-camelCase"
  refactor on EITHER side silently breaks the contract.

## What breaks
- Any encoding/case mismatch → serde deserialize failure → guardian returns
  400/422, reef logs a provisioning failure → validator never provisioned
  (liveness). Fails closed (no wrong-key risk), but silent and version-fragile.

## Property statement
`Always`: a `ValidateCustodyRequest`/`Response` serialized by one side
deserializes losslessly on the other (round-trip identity). In-binary form:
`Always(serde_json::from_str(serde_json::to_string(req)) == req)` for generated
requests, AND the `guardian_enclave_public_key` field is base64 while all
sig/key/root fields are hex (assert the encoding of each field).

## Antithesis angle
- Mostly deterministic round-trip / input-coverage: generate diverse valid
  requests (varied key bytes, varied vec lengths, 0x-prefixed and bare hex) and
  assert lossless round-trip. `Always`.
- Version-compat is better caught at CI/build than at runtime, but Antithesis
  catches the *behavioral* symptom: `Sometimes(a well-formed request deserializes
  and reaches verify_custody)` — if never reachable, the wire contract is broken.
- Low fault-injection content; flag as primarily a wire-contract/regression
  property.

## Instrumentation status: MISSING
No serde round-trip assertions. Cheap to add an `assert_always` round-trip in a
shared (de)serialization helper. The base64/hex asymmetry deserves an explicit
`assert_always(guardian_enclave_public_key field is base64)` so a future serde
change trips it.

## Open questions
- Does reef pin the exact same `ecies`/`libsecp256k1` as the guardian? **Largely
  resolved:** reef pulls `puffersecuresigner` (the SUT) as a git dep
  (`reef-guardian/Cargo.toml:39`, `reef-lib/Cargo.toml:36`, branch
  `feat/tdx-improve-signature`) and uses the SUT's own `ecies::PublicKey`
  (`EthPublicKey`) type for the `guardian_enclave_public_key` field — so the serde
  impl is shared by construction, not merely "likely". Divergence is only possible
  if the two repos resolve different transitive `ecies`/`libsecp256k1` versions in
  Cargo.lock; the pinned `ecies = "=0.2.7"` makes that unlikely.
- Is the mixed camelCase/snake_case intentional and frozen, or tech debt slated
  for cleanup (which would break the contract)? *(needs human input)*

---

## Investigation Log

### Does reef store/compare the compressed or uncompressed guardian enclave pubkey when it calls validate-custody? Does the keygen→config→custody round-trip ever mismatch?
- **Examined:**
  - `reef/reef-guardian/src/handlers/api/guardian/rotate_guardian_key.rs` (keygen
    call + what it returns/implies for storage), esp. `:88-136, :191-234`.
  - `reef/reef-guardian/src/handlers/api/webhooks/provision_or_skip/new_registration.rs:193-272`
    (`generate_validate_custody_request`).
  - `reef/reef-guardian/src/config/guardian.rs` +
    `constants/{staging,production}/guardian.json` (where the stored pubkey lives).
  - SUT: `src/enclave/types.rs:17-24` (keygen returns uncompressed),
    `src/crypto/eth_keys.rs:27-30` (`eth_pk_to_hex` = compressed), `:87-95`
    (`save_eth_key`), `:98+` (`fetch_eth_key`),
    `src/enclave/guardian/mod.rs:64-68` (custody lookup re-compresses).
- **Found:**
  - **reef stores the UNCOMPRESSED form.** The config `enclave_public_key` is a
    65-byte uncompressed key: staging `04097a98…` and production `046b1b5c…` are
    both 130 hex chars with the `04` uncompressed prefix
    (`constants/*/guardian.json`). rotate_guardian_key reinforces this: it returns
    `eth_public_key.serialize().to_vec().encode_hex()` (=`ecies::PublicKey::serialize`
    = uncompressed 65B) in `ResponseBody.eth_public_key` (`:212, :230`), matching
    the SUT keygen which returns `pk.serialize()` uncompressed (`types.rs:19`).
  - **reef sends the parsed point, and the parse auto-detects.**
    `new_registration.rs:205-230`: it `hex::decode`s the config pubkey and calls
    `EthPublicKey::parse_slice(&bytes, None)` — `None` = auto-detect by length, so
    the uncompressed 65B config value parses fine to the curve point. That point
    goes into `ValidateCustodyRequest.guardian_enclave_public_key` (`:264`).
  - **The SUT re-compresses on lookup**, bridging the split:
    `mod.rs:64-65` `fetch_eth_key(&eth_pk_to_hex(&request.guardian_enclave_public_key))`
    where `eth_pk_to_hex` = `serialize_compressed()` (`eth_keys.rs:28-29`). The file
    was written under the same compressed form (`save_eth_key` → `eth_pk_to_hex`,
    `:87-88`). So the on-disk filename (compressed) and the lookup key (compressed)
    match **because the SUT re-compresses whatever curve point reef sent**, and reef
    faithfully preserves that point through uncompressed-hex → `parse_slice` →
    serde.
- **Not found:** Any reef path that stores or transmits the *compressed* form, or
  any place reef string-compares the keygen-returned pubkey against the list
  endpoint (`/eth/v1/keygen` GET). reef never calls `list_eth_keys` for identity
  matching in the provisioning/eject flow.
- **Conclusion:** reef round-trips the **uncompressed** form end-to-end (keygen
  returns uncompressed → reef config holds uncompressed → reef sends the parsed
  point). The split does **not** bite in the current reef flow precisely because
  `fetch_eth_key`/`eth_pk_to_hex` re-compresses the point on the SUT side, and
  reef's `parse_slice(None)` + the shared `ecies::PublicKey` serde preserve the
  exact curve point. The hazard is therefore latent, not live: it would only bite
  under encoding drift (a future caller passing a form whose parse/serde doesn't
  reconstruct the same point) or a divergent `ecies`/`libsecp256k1` version. This
  resolves the open question of whether the split bites in the reef flow — **it
  does not today; the SUT-side re-compression is exactly what makes it work**, which
  argues for *asserting* the round-trip rather than removing the split.

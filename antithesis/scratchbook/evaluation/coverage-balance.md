---
sut_path: /home/fawad/puffer/projects/validator-guardian
commit: f7dbd88d99de21a6d8caba5f7de8216624ab1098
updated: 2026-06-30
external_references:
  - path: /home/fawad/puffer/projects/reef
    why: consumer call patterns
  - path: /home/fawad/puffer/projects/coral
    why: confirms coral does not call the guardian
---

# Coverage Balance Evaluation — `guardian` binary property catalog

Lens: adversarial portfolio review. Question is "is this the *right set* of 27
properties?" — gaps, disproportion, missing TYPES, cross-cutting concerns that
fall between focuses, and component blind spots — not "is each property good?"

Method: walked `sut-analysis.md` §2–§13 area-by-area against the 27 cataloged
properties (verified all 27 evidence files survived synthesis — present and
non-empty), then re-derived the route/component surface and the keygen-evidence
downstream path from the SUT source and the reef consumer.

---

## Portfolio snapshot (what the set actually covers)

| SUT area (sut-analysis §) | Properties | Balance |
|---|---|---|
| Attestation / session verify (§5,§6 G1,§7 F1/F10, A) | attestation-verified-before-approval, session-verification-fails-closed, cvm-stub-never-in-production | covered (inert today) |
| Preimage/byte agreement (§8,§9 W0, B) | custody-preimage-matches-onchain-verifier, rotate-key-preimage-matches-onchain, attestation-payload-reconstruction-matches, session-public-key-parse-no-wrong-identity, pubkey-representation-consistent | well covered |
| Custody well-formedness S1–S4 (§6, C) | stored-share-matches-committed-pubkey-share, custody-rejects-invalid-deposit-data, fork-version-source-consistency | covered |
| Approval binding/replay (§9 W1–W3, D) | approval-binding-and-replay-resistance, approval-deterministic-and-idempotent | covered |
| Sign-exit auth/binding (§9 W10, E) | sign-exit-requires-authorization, sign-exit-index-binding | covered |
| Persistence/crash (§3,§4, F) | key-write-durable-or-rejected, no-orphaned-unacknowledged-secret, torn-read-never-yields-wrong-key, missing-state-request-fails-clean | well covered |
| Liveness/availability (§4,§7 F3, G) | dependency-hang-makes-progress, upcheck-live-under-load, bounded-resource-usage | covered |
| Input robustness/lifecycle (§7 F7/F8/F11, H) | malformed-input-never-panics, startup-bind-failure-not-silent, guardian-routes-time-independent, no-request-authentication | covered |

Per-component route coverage of the four-route surface (`bin/guardian.rs:35-65`):

| Route | Direct properties | Verdict |
|---|---|---|
| `POST /guardian/v1/validate-custody` | ~14 of 27 touch it | concentration point |
| `POST /eth/v1/keygen` (produce) | rotate-key-preimage, cvm-stub, key-write (ETH branch), bounded-resource, malformed-input | thin given it is the root-of-trust producer |
| `POST /guardian/v1/sign-exit` | sign-exit-requires-authorization, sign-exit-index-binding, torn-read, key-write (BLS read) | covered |
| `GET /eth/v1/keygen` (list) | malformed-input (F11 only) | under-covered (see findings) |
| `GET /upcheck` | upcheck-live-under-load, startup-bind-failure-not-silent | covered |

The type mix is **Safety 16 / Liveness 4 / Reachability 7**. Given the SUT's
own framing — the no-timeout / head-of-line / no-spawn_blocking story is one of
the most realistic *and always-live* fault classes (§4, §7 F3, P1 makes the
attestation safety props inert) — liveness at 4 is defensible but on the low
side; see CW-2.

---

## Findings (catalog-wide first, then property-scoped)

### CW-1 — Keygen→reef→on-chain `rotateGuardianKey` consumption of the evidence struct is only half-covered (cross-cutting, between focuses)

- **Property/ies:** catalog-wide; nearest is `rotate-key-preimage-matches-onchain`.
- **Concern:** The task asks specifically whether the *keygen attestation*
  (guardian-side, ROTATE_GUARDIAN_KEY) interaction with downstream consumption
  is covered. The keygen handler returns a `KeyGenResponse { pk_hex, evidence }`
  where `evidence: AttestationEvidence { session_id, signature,
  session_public_key, owner_public_key }` (`remote_attestation.rs:14-20`). reef
  `rotate_guardian_key.rs:88-195` consumes **all four evidence fields** plus
  `pk_hex`, repackages them into a `GuardianSessionProof { session_id,
  session_key, owner_key, signature }`, and submits them to
  `GuardianModule.rotateGuardianKey()`, which verifies via
  `SessionRegistry.verifySessionSignature()`. So the keygen evidence is a
  *live, on-chain-verified output today* — unlike the custody attestation path
  (P1, inert). `rotate-key-preimage-matches-onchain` covers only the **keccak
  preimage** (the `signedMessageHash`). It does **not** cover the rest of the
  proof the contract checks: (a) the `PublicIdentity` `typeId`/`key`
  serde+ABI round-trip for **`owner_public_key`** (the contract requires
  `ownerKey.typeId == ALGO_ID_ES256K` and `ownerKey.key.length == 65`,
  GuardianModule.sol:316-317) — `session-public-key-parse-no-wrong-identity`
  only covers the *validate-custody* `parse_session_public_key` path, a
  different code path that never runs on keygen; (b) the `pk_hex` 65-byte
  uncompressed→`ecies::PublicKey::parse` round-trip reef does at `:118-135`
  (the keygen analogue of `pubkey-representation-consistent`, which is scoped
  only to validate-custody's compressed-filename lookup). Net: the catalog's
  highest-severity on-chain liveness property (`custody-preimage-...`) has a
  twin on the **keygen** side that is *actually live in production today* and is
  only partially covered. If the deployed contract rejects the rotate proof,
  the guardian can never register its enclave key on-chain → it is excluded from
  quorum → the same protocol-wide-stall blast radius as W0, but reachable
  *now*, with attestation "off."
- **Scope:** catalog-wide (the keygen evidence lifecycle).
- **Evidence:** `remote_attestation.rs:14-20,30-43`;
  `enclave/types.rs:11-32` (`KeyGenResponse::from_eth_key`);
  reef `rotate_guardian_key.rs:88-195` (consumes evidence + submits on-chain);
  `rotate-key-preimage-matches-onchain.md` (covers only the keccak preimage).
- **Suggested action:** Add a property (or widen `rotate-key-preimage-...`)
  asserting the full keygen→`rotateGuardianKey` round-trip clears against the
  deployed `GuardianModule` in the anvil variant: `Reachable("keygen evidence
  accepted by rotateGuardianKey on-chain")` plus `Always("keygen owner/session
  PublicIdentity round-trips to typeId=3, 65-byte key")`. This is the keygen
  counterpart to the custody-preimage critical property and deserves comparable
  weight, especially since it is *not* gated by P1.

### CW-2 — Liveness under-weighted for the keygen path specifically; `dependency-hang` is the only keygen-liveness property and its RPC variant is inert

- **Property/ies:** `dependency-hang-makes-progress`, `upcheck-live-under-load`.
- **Concern:** The SUT's most realistic always-live fault is dependency hang /
  head-of-line blocking (§4, §7 F3; no timeouts confirmed, ECIES loop O(N) on
  the worker, no `spawn_blocking`). Only **one** property
  (`dependency-hang-makes-progress`) directly targets a hung dependency, and its
  two triggers split unevenly: the **CVM-agent hang** (keygen + validate-custody)
  is always-live, but the **SessionRegistry RPC hang** is only reachable when
  `verify_session=true` (P1 — inert today, noted in that property's own Open
  Questions). So in the production-representative configuration there is exactly
  one always-live keygen-liveness trigger (CVM socket hang) and it is shared with
  custody. Given that a hung CVM agent stalls reef's *entire* provisioning step
  with no recovery (that property's resolved OQ confirms reef sets no timeout),
  4 liveness properties is thin relative to the risk. The catalog correctly
  flags this, but the portfolio leans Safety-heavy (16) where the live-today
  blast radius is concentrated in liveness.
- **Scope:** `dependency-hang-makes-progress` (RPC variant) + the liveness
  category as a whole.
- **Evidence:** catalog §"no timeouts anywhere"; `dependency-hang-makes-progress.md`
  Open Questions ("RPC-hang variant only live when verify_session=true");
  `upcheck-live-under-load.md`.
- **Suggested action:** No new property strictly required, but ensure the harness
  spends real budget on the CVM-agent `hang` mode against **keygen** (not just
  custody), since keygen-on-rotate is the live root-of-trust path. Consider an
  explicit liveness assertion that keygen returns terminal status under CVM hang,
  distinct from the custody trigger.

### CW-3 — `GET /eth/v1/keygen` (list) behavior covered only for F11 panic, not for list-contract / readiness semantics

- **Property/ies:** `malformed-input-never-panics` (F11 portion),
  `missing-state-request-fails-clean`, `pubkey-representation-consistent`.
- **Concern:** The task explicitly asks whether `GET /eth/v1/keygen` is covered
  beyond F11. It is not. `malformed-input-never-panics` covers the
  `ListKeysResponse::new` `pk[0..2]` panic on a short/odd filename
  (`types.rs:50`). But three other list behaviors are uncovered: (a) the
  empty-dir case returns **500** (`list_fnames` → `fs::read_dir` errors "No keys
  saved in dir" → `read_key`/handler 500), which `missing-state-request-fails-clean`
  flags as an Open Question ("should GET return `[]` rather than 500?") but does
  not assert; (b) the list returns the **compressed** filename while keygen
  returned the **uncompressed** pubkey — `pubkey-representation-consistent` names
  this three-faces footgun but its invariant is scoped to the *validate-custody*
  lookup, not to asserting list↔keygen↔disk agreement on the GET path; (c)
  there is no readiness/health tie-in (a guardian with an empty eth_keys dir
  still returns `/upcheck` 200 — `startup-bind-failure-not-silent` mentions "no
  keys" but its angle is bind-failure/silent-exit, not the list endpoint).
- **Scope:** GET-list endpoint coverage; cross-refs `pubkey-representation-consistent`.
- **Evidence:** `key_management.rs:117-149` (list_fnames, 500 on empty dir);
  `types.rs:44-62` (`ListKeysResponse::new`, `pk[0..2]`);
  `missing-state-request-fails-clean.md` Open Question; `eth_keys.rs:28-35`
  (compressed vs uncompressed split).
- **Suggested action:** Either fold an explicit "GET list returns structured
  output (or clean error) for empty/populated/crafted dirs, and the listed value
  re-derives the keygen-returned key" assertion into
  `pubkey-representation-consistent` / `missing-state-request-fails-clean`, or add
  a small list-contract property. Low severity, but it closes the GET blind spot.

### CW-4 — Concentration on validate-custody is appropriate, but the ETH-key *produce* path is thin relative to its root-of-trust role

- **Property/ies:** keygen-path properties as a group.
- **Concern:** ~14 of 27 properties touch validate-custody; the keygen POST
  (the *producer* of the enclave ETH key and the attestation that
  `rotateGuardianKey` consumes) is touched by only ~5, and three of those are
  shared-mechanism (key-write ETH branch, bounded-resource, malformed-input)
  rather than keygen-specific. The concentration on validate-custody is justified
  (it is where S1–S6, the persistence window, and most attacker-controlled fields
  live), so this is **not** over-investment in custody — it is *under*-investment
  in keygen given that (per CW-1) keygen's output is the one attestation path
  that is on-chain-verified *today*. The ETH-key write-before-attestation
  ordering (`mod.rs:31` writes sk before `:56` awaits the CVM agent) is covered by
  `no-orphaned-unacknowledged-secret` and `key-write-durable-or-rejected` generally,
  but there is no keygen-specific "fresh ETH key lost on crash is unrecoverable
  (no backup)" liveness witness distinct from the BLS-share orphan story.
- **Scope:** keygen POST path.
- **Evidence:** route table `sut-analysis.md §2`; `mod.rs:29-58`;
  `no-orphaned-unacknowledged-secret.md` (covers both keygen+custody writes but
  frames the harm via the BLS share / sign-exit chain).
- **Suggested action:** Confirm the harness exercises the keygen crash window
  (kill between `eth_key_gen()` write and `AttestationEvidence::new` await) and
  the resulting unrecoverable-key state explicitly, not only the custody-share
  orphan. No new property required if the existing two are driven on keygen.

### F-1 — base64/hex serde *deserialization-failure* boundary is asserted as round-trip success, not as a fail-closed deserialization barrier

- **Property/ies:** `pubkey-representation-consistent`, `malformed-input-never-panics`.
- **Concern:** The task asks whether the base64/hex serde boundary
  (deserialization *failures*) is covered. `guardian_enclave_public_key`
  deserializes as **base64** via libsecp256k1's `serde` impl while every other
  field is hex (`sut-analysis.md §2`; `types.rs:87`
  `guardian_enclave_public_key: EthPublicKey`). `pubkey-representation-consistent`
  asserts the *happy round-trip* (returned uncompressed → compressed filename),
  and its investigation concluded the split is "latent not live." But the
  **failure** mode — a client/version that sends hex where base64 is expected (or
  vice versa) → axum `Json` extractor rejects with 422/400 *before the handler
  runs* — is only mentioned in `sut-analysis.md §2` ("silently fails to
  deserialize → 422/400") and is not asserted anywhere. This is distinct from
  the F7/F8/F11 *post-deserialization* length panics that
  `malformed-input-never-panics` covers (those happen inside the handler on hex
  fields, not on the base64 field). The gap: no property witnesses that a
  malformed/wrong-encoding `guardian_enclave_public_key` yields a clean 4xx and
  never a panic or a confused-identity 500.
- **Scope:** `pubkey-representation-consistent` + `malformed-input-never-panics`.
- **Evidence:** `types.rs:83-93` (`ValidateCustodyRequest`, base64 `EthPublicKey`
  field amid camelCase hex fields); `sut-analysis.md §2` serde boundary note;
  `malformed-input-never-panics.md` (panic inventory F7/F8/F11 is
  post-deserialization, hex-only).
- **Suggested action:** Extend `malformed-input-never-panics` (or
  `pubkey-representation-consistent`) with a `Sometimes("validate-custody rejected
  a wrong-encoding guardian_enclave_public_key with a structured 4xx")` and assert
  no panic/connection-reset on the JSON-extractor boundary. Low-medium severity;
  it is the one serde boundary the panic inventory does not reach.

### F-2 — Clock faults are used only as a negative control; no positive use, which is correct (a pass with a caveat)

- **Property/ies:** `guardian-routes-time-independent`.
- **Concern:** The task asks whether clock faults are used anywhere besides the
  time-independence property. They are not, and per the SUT this is **correct**:
  no guardian route reads wall-clock time (epoch hardcoded 0, no `tokio::time`,
  no timeouts), so there is no positive clock-dependent behavior to test. The one
  *latent* clock dependency — session `expiresAt` / `isActive`, which
  `verify_session_evidence` never reads (§7 F10) — is owned by
  `session-verification-fails-closed`, not by a clock-fault property, which is the
  right home. The single caveat: if `verify_session` is ever turned on and
  `expiresAt` logic is added, the negative-control property silently becomes
  insufficient and `session-verification-fails-closed` would need a clock-advance
  fault (expire a registered session, assert reject). That dependency is noted in
  `property-relationships.md` ("ignored-clock link") but not as an action item on
  `session-verification-fails-closed`.
- **Scope:** `guardian-routes-time-independent` / `session-verification-fails-closed`.
- **Evidence:** `mod.rs:115-225` (no isActive/expiresAt read);
  `session_registry.rs:30-45`; `guardian-routes-time-independent.md`;
  `property-relationships.md` cross-cluster "ignored-clock link."
- **Suggested action:** When `verify_session` is enabled, add a clock-advance
  fault to `session-verification-fails-closed` (expire a session → assert
  reject). Until then, current single-use is correct — flag, do not add now.

---

## Passes (areas the portfolio covers well; verified, not gaps)

- **`/upcheck` readiness-vs-liveness gap is covered.** The task flags this risk;
  it is split correctly across `upcheck-live-under-load` (head-of-line liveness)
  and `startup-bind-failure-not-silent` (the aspirational `Always("/upcheck 200
  ⇒ guardian can serve")`, explicitly a readiness-vs-liveness split). Confirmed
  against `health.rs` (unconditional 200) and `bin/guardian.rs:69` (discarded
  bind result). No gap.
- **Slash-protection DB out-of-scope for the guardian is correctly excluded.**
  Verified: the guardian router (`bin/guardian.rs:35-65`) registers no
  `/api/v1/eth2/sign` route; `eth2/slash_protection.rs` and `shared/mod.rs`
  deposit-forbidden/slashing checks are reachable only via the
  validator/secure-signer routers (sut §6 G4). The catalog spends zero budget
  there — correct (sut §4 confirms guardian routes never touch the slash DB).
  Not a missed surface.
- **The inert attestation path (Category A/B verify-path) is *correctly*
  invested in, not over-invested.** All on-the-verify-path properties are framed
  honestly (`Reachable` to document the bypass today + aspirational `Always` for
  when attestation is mandatory), and P1 is loud across the catalog. Given the
  SUT's bug-history (§11 — the entire attestation path was rewritten SGX→TDX in
  the last 4 months, untested, freshest/riskiest code) and the explicit signal
  that `verify_session` is "about to be turned on," testing the inert path now is
  the right call. The catalog does **not** over-rotate budget into it relative to
  the always-live custody/persistence/availability paths — those (Categories
  C/F/G) carry 10 properties including the strongest holdable invariant
  (`stored-share-matches-committed-pubkey-share`) and the confirmed escalations
  (BLS zero-key fail-open). Balance here is good.
- **The catastrophe chain is covered as a portfolio, not just per-property.**
  attestation-off (P1) → orphaned unattested share (`no-orphaned-...`) →
  live exit capability (`sign-exit-requires-authorization`) is wired explicitly
  in `property-relationships.md` and each link is its own property. The
  highest-value end-to-end timeline is not falling between focuses.
- **W0 dual-bind (custody 5-field vs rotate-key 7-field, no single contract
  branch satisfies both) is covered from both ends** —
  `custody-preimage-matches-onchain-verifier` (custody) and
  `rotate-key-preimage-matches-onchain` (rotate). The portfolio captures the
  *pair*, which is what makes the dual-bind visible.
- **Concurrency / non-atomic-write surface is well covered** for the
  always-live paths: `torn-read-never-yields-wrong-key`,
  `key-write-durable-or-rejected`, `no-orphaned-unacknowledged-secret`,
  `missing-state-request-fails-clean` cover the four faces (concurrent reader,
  crash, orphan, missing-state) of the single `fs::write`-no-fsync mechanism, and
  share the blsttc zero-key fail-open escalation.

---

## Uncertainties (need human input or out-of-scope to resolve here)

- **CW-1 severity hinges on which contract bytecode is deployed (catalog Q1).**
  If the deployed `GuardianModule` is `feat/tdx`, rotate-key byte-matches and the
  keygen on-chain path works today; if it is `fix/increase-guardian-signatures-security`,
  rotate-key uses the old format and the keygen path is broken now. The catalog
  already flags Q1 as the single most important unknown; CW-1's "live today"
  framing assumes `feat/tdx` is deployed. Cannot resolve from code.
- **Whether `verify_session` will be flipped on (Q3)** governs whether CW-2's RPC
  hang variant, F-2's expiry caveat, and the entire Category-A/B verify-path move
  from inert to live. Product decision.
- **Whether the keygen evidence `owner_public_key`/`session_public_key` can ever
  diverge in encoding between the real DCAP agent and the contract** — the
  in-repo atakit sim emits the 65-byte uncompressed form that matches, but the
  production DCAP binary is closed-source (per `rotate-key-preimage-...` resolved
  OQ). CW-1's PublicIdentity round-trip property would test the sim path; the real
  agent's bytes remain human-input.
- **Did I miss a fifth route or a hidden guardian-reachable handler?** Verified
  the router registers exactly four routes + upcheck; the per-component table is
  derived from that. If the deployment fronts the guardian with a proxy that adds
  routes, that surface is out of scope for this evaluation.

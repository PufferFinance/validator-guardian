---
sut_path: /home/fawad/puffer/projects/validator-guardian
commit: f7dbd88d99de21a6d8caba5f7de8216624ab1098
updated: 2026-06-30
external_references:
  - path: /home/fawad/puffer/projects/reef
    why: reef-guardian is the sole real consumer; pins which request fields are populated (verify_session=false, workload_id="", guardian_index=0) and confirms each guardian submits its own provision_node on-chain (no off-chain aggregation).
  - path: /home/fawad/puffer/projects/coral
    why: confirmed coral does NOT call the guardian (it talks to the validator binary); reframes the trust boundary onto reef/BFF + puffer-contracts.
---

# Property Catalog — `guardian` binary

> Scope: the `guardian` binary only. Priority area: **attestation / signature
> verification** (Categories A–B, plus E). Derived from a 7-focus discovery
> ensemble; per-property evidence in `properties/{slug}.md`; clusters in
> `property-relationships.md`. **29 properties** in 8 categories (27 from discovery
> + 2 gap-fills added in the evaluation pass: `keygen-evidence-accepted-onchain`,
> `unattested-share-cannot-become-exit-capability`).

## How to read this catalog

- **Assertion types** are Antithesis SDK assertions (`Always`,
  `AlwaysOrUnreachable`, `Sometimes(cond)`, `Reachable`, `Unreachable`) — they
  report outcomes and guide search; they do **not** crash the program. **None
  exist yet** (`existing-assertions.md`); every instrumentation note below is
  **MISSING** unless stated otherwise, and the `antithesis-sdk` crate must be
  added before SUT-side assertions compile.
- **Open Questions** under each property list the *remaining* unresolved
  questions, tagged `(partial: …)` / `(needs human input)`. As a deliberate
  (non-default) deviation from the skill convention's "remove resolved questions",
  questions the investigation pass *answered* are retained as a compact
  `~~struck-through~~ **RESOLVED**` marker for at-a-glance delta visibility; the
  full audit trail lives in each evidence file's `Investigation Log` and the
  "Investigation pass results" summary above. Treat struck bullets as closed.

## Catalog-wide preconditions (read first — these gate severity)

These are not properties; they are facts that change how several properties
should be interpreted, and they need human/product confirmation
(see `sut-analysis.md` Open Questions):

- **P1. Attestation is OFF in production today.** reef sends `verify_session:
  false` with empty `session_id`/`attestation_signature`/`session_public_key`/
  `workload_id` (`new_registration.rs:257-266`). So Category-A/B properties on
  the on-chain verification path are **inert in production** unless/until
  `verify_session` is flipped on. Properties on that path are framed honestly:
  `Reachable` to document the bypass *today*, with an aspirational `Always`
  guardrail for *if attestation becomes mandatory*.
- **P2. The guardian trusts an unauthenticated caller.** No request
  authentication on any endpoint (see [[no-request-authentication]]). Whether
  `validate-custody` / `sign-exit` are reachable by untrusted clients in the TDX
  topology (Q4) sets the severity of every "attacker-controlled field" property
  from *robustness* to *remotely exploitable*.
- **P3. Same binary on all guardians.** A *systematic* flaw is correlated across
  the M-of-N set → catastrophic at quorum; a single guardian's liveness failure
  is tolerated below threshold.
- **P4. On-chain aggregation.** Each guardian independently validates, signs, and
  submits its own `provision_node`; `GuardianModule`/`PufferProtocol` is the
  threshold aggregator. There is no off-chain signature aggregation.

## Faults required (flag for tenant config)

Node **termination/restart is disabled by default** on most Antithesis tenants.
These properties REQUIRE it (confirm enabled, else they pass vacuously):
[[key-write-durable-or-rejected]], [[no-orphaned-unacknowledged-secret]]
(crash variant), [[missing-state-request-fails-clean]] (restart variant).
Clock faults are needed only as a negative control for
[[guardian-routes-time-independent]].

## Investigation pass results (2026-06-30)

A four-agent investigation pass resolved the code-answerable open questions
(evidence files carry the full `### Investigation Log`s). Headlines:

- **Escalation (safety):** the on-disk BLS read path `SecretKeySet::from_bytes`
  → `Poly::from_bytes` **fails OPEN** — empty *or* even-hex-but-<32-byte content
  yields a **zero key with no error** (only the fixed-length `[u8;32]` decrypt
  path fails closed). This promotes [[key-write-durable-or-rejected]] (BLS branch)
  and [[torn-read-never-yields-wrong-key]] from liveness/flakiness to **silent
  wrong-key safety**, reachable from just the `O_TRUNC` window.
- **Confirmed High:** reef **always** sends `guardian_index = 0` (sole call site,
  no per-guardian index config) ⇒ [[sign-exit-index-binding]] is a real
  silent-mute bug for any guardian whose share decrypts at index ≠ 0.
- **Determinism holds:** locked `ethers` is **2.0.14** (k256 0.13.4) ⇒ RFC-6979
  deterministic `k`, canonical low-S — [[approval-deterministic-and-idempotent]]
  stands; [[sign-exit-requires-authorization]] is byte-for-byte replayable.
- **W0 dual-bind confirmed in code:** `feat/tdx` verifies a **5-field** custody
  preimage (no match); the **7-field** match exists only on
  `origin/fix/increase-guardian-signatures-security`, which lacks the
  `ROTATE_GUARDIAN_KEY` format `feat/tdx` has. **No single branch satisfies both.**
  Which bytecode is *deployed* remains the one human-input unknown.
- **Downgrades:** [[pubkey-representation-consistent]] is **latent not live** (reef
  round-trips the uncompressed form, shares the `ecies` type); W9 /
  [[session-public-key-parse-no-wrong-identity]] is **liveness-only** (registry
  fails closed on bad typeId, and the in-repo producer emits raw hex so the JSON
  branch is dead for SUT payloads).
- **Reconstruction matches by construction:** the validator-side attestation
  payload is built by the *same* `build_validator_remote_attestation_payload`
  (`usize`→8 bytes on x86_64) ⇒ [[attestation-payload-reconstruction-matches]]
  confidence raised to High; residual risk is the `.zip` desync + version skew.
- **No timeouts anywhere / no panic-or-body middleware confirmed**, and the ECIES
  trial-decrypt loop is **O(N) in an uncapped caller-controlled vector** ⇒
  [[dependency-hang-makes-progress]], [[upcheck-live-under-load]],
  [[bounded-resource-usage]], [[malformed-input-never-panics]] all strengthened;
  panic inventory is exactly F7/F8/F11.

## Property-type / priority distribution

- Safety: 18 · Liveness: 4 · Reachability/Documenting: 7  *(29 total)*
- Critical: 2 · High: 16 · Medium: 8 · Low: 3 *(post-investigation + evaluation: +2 gap-fills [High]; approval-deterministic elevated Medium→High; session-public-key-parse downgraded to Low)*

---

## Category A — Attestation & session verification *(priority)*

Covers whether the guardian actually verifies the validator-enclave attestation
before it co-signs custody, and whether that verification is sound when it runs.

### attestation-verified-before-approval — No custody approval (or stored share) without verified attestation
| | |
|---|---|
| **Type** | Safety (today: Reachability documenting the bypass) |
| **Property** | The guardian must not emit a custody approval — nor persist a share usable by sign-exit — for a payload whose CVM session/attestation was not verified. |
| **Invariant** | Today, honest encoding is `Reachable("custody approval emitted with verify_session=false")` — an `Always(approval ⇒ session_verified)` would *fail on the real workload* (reef sends false), so it can only be an aspirational guardrail for when attestation is mandatory. The cross-cutting master form (w7-6) tags each persisted share at write with "was verification run?" and asserts at sign-exit `Always(exit_signed ⇒ share_was_verified)`. `Reachable`/`Always` chosen because this is a binary "did the gate run" fact, not a liveness state. |
| **Antithesis Angle** | Workload sends `verify_session` ∈ {true,false}; assert which branch ran and whether an approval/exit nonetheless resulted. Combine with the persistence + sign-exit lifecycle to witness an unattested share becoming a live exit capability. |
| **Why It Matters** | This is the guardian's reason to exist. Bypass replicated across ≥M guardians forges provisioning of an unattested validator (un-ejectable, funds stuck) or, via sign-exit, force-ejects live validators. The #1 documented gap (F1 + W10 + W6). |
| **Priority** | Critical |
| **Confidence** | High — `mod.rs:71` (client-controlled gate), reef `new_registration.rs:266`, persistence `mod.rs:82-85` before approval `:88`, ungated `sign_voluntary_exit_message` `:352-370`. |

**Open Questions:**
- Is `verify_session` intended to be flipped on in production? If yes, reef's `false` is a live bug and the `Always` guardrail applies; if no, the entire SessionRegistry path is aspirational and this is a documenting `Reachable`. *(governs the whole category)*
- Is `sign-exit` reachable by untrusted clients (Q4)? Sets severity of the master-invariant violation.

### session-verification-fails-closed — When attestation IS verified, it fails closed and checks session liveness
| | |
|---|---|
| **Type** | Safety |
| **Property** | On the `verify_session=true` path, an approval is emitted only if `SessionRegistry.verifySessionSignature` returns true AND the session is active/unexpired AND (when supplied) the workload matches. |
| **Invariant** | `Always("approval on verify-path ⇒ registry returned valid")` (fail-closed on any RPC error — currently true, errors → 500). Plus a gap to close: `verify_session_evidence` never reads `session.isActive`/`expiresAt` (`session_registry.rs:30-45`) and skips the workload check when `workload_id` is empty — add `Always("verified session is active and unexpired")`. `Always` fits because any execution of the path must satisfy it. |
| **Antithesis Angle** | Mock SessionRegistry RPC; inject network faults/garbage/timeouts on the eth_call — assert never-approve-on-error (fail-closed). Feed expired/revoked sessions to exercise the missing isActive/expiresAt check. |
| **Why It Matters** | If attestation is turned on, accepting expired/revoked/wrong-workload sessions reintroduces the bypass through the front door (F10). |
| **Priority** | High *(conditional on P1: inert until verify_session=true)* |
| **Confidence** | High — `mod.rs:115-225`, `session_registry.rs`. |

**Open Questions:**
- Same as the category: is the verify path live? *(needs human input / product decision)*
- Is an empty `workload_id` acceptable when `verify_session=true`, or should it be rejected? *(decides whether the skipped workload-match is a bug)*
- Requires a mock SessionRegistry in the harness (env unset by default ⇒ path otherwise unreachable).

### cvm-stub-never-in-production — CVM_AGENT_STUB must never silently disable attestation in prod
| | |
|---|---|
| **Type** | Safety / config (Reachability of the stub branch) |
| **Property** | A keygen `201` must mean genuine CVM-signed evidence; `CVM_AGENT_STUB=true` returning `AttestationEvidence::default()` (empty) must be impossible in a production image. |
| **Invariant** | `Reachable("keygen returned empty stub attestation")` to document the branch; recommended `Unreachable("production keygen returned empty attestation evidence")` once a prod/non-prod signal exists. `Unreachable` fits a state that must never occur in prod. |
| **Antithesis Angle** | Config-driven: toggle the env var; assert a 201 can carry empty evidence and that nothing downstream notices. |
| **Why It Matters** | A mis-stubbed image collapses the root of trust while looking healthy (W11/F3). |
| **Priority** | Medium *(LIVE: gated by `CVM_AGENT_STUB`, not `verify_session`; config/regression-tier — see evaluation Bias B1)* |
| **Confidence** | High — `remote_attestation.rs:30-34`; committed `env` sets neither the stub nor `SESSION_REGISTRY_*`. |

**Open Questions:**
- Can `CVM_AGENT_STUB=true` reach a production image (does the build/deploy strip it)? *(needs build audit, Q5)*

---

## Category B — Signature & preimage correctness *(priority)*

The guardian hand-rolls three signed/verified byte-preimages; each must match an
external verifier (Solidity contract, or the off-line Automata CVM signer)
byte-for-byte. Drift = silent verification failure (liveness) or a forgery seam.

> **W0 dual-bind (evaluation R3):** [[custody-preimage-matches-onchain-verifier]]
> (7-field custody) and [[rotate-key-preimage-matches-onchain]] (ROTATE_GUARDIAN_KEY)
> are **not independent** — no single deployed `puffer-contracts` branch satisfies
> both (the 7-field custody verifier is only on `fix/increase-guardian-signatures-security`,
> which lacks the rotate format on `feat/tdx`). Their conjunction is currently
> **unsatisfiable**; test them as a cross-process XOR, not two independent oracles.

### custody-preimage-matches-onchain-verifier — Guardian custody preimage byte-matches the on-chain verifier
| | |
|---|---|
| **Type** | Safety (cross-process agreement) |
| **Property** | The digest the guardian signs in `approve_custody` equals `keccak256(abi.encode(...))` of the field set the deployed `GuardianModule`/`LibGuardianMessages` verifies, so a threshold of honest guardian approvals clears on-chain. |
| **Invariant** | `Always("recovered signer == enclave address")` (S5, already a `bail!`) + an `Always` that the preimage field count/order matches a pinned reference encoder. Full agreement needs the contract (or a reference encoder) in the harness. `Always` because every signed approval must satisfy it. |
| **Antithesis Angle** | Generate diverse `chainId`/`validatorIndex`/`address`/`pubKey`/`sig`/`wc`; verify against a reference encoder and (ideally) a forked-chain `GuardianModule`. **Primarily input-coverage + regression detection**, not fault injection. |
| **Why It Matters** | **W0, confirmed byte-for-byte:** the guardian signs a **7-field** preimage (`mod.rs:322-336`) but `puffer-contracts@feat/tdx` `LibGuardianMessages` verifies only **5 fields** → every guardian's correct signature is rejected on-chain → **provisioning stalls protocol-wide today**. Commit `61bf1d2` shows this drifted before. Catastrophic correlated liveness. |
| **Priority** | Critical |
| **Confidence** | High — drift confirmed against the on-disk contract checkout; comment at `mod.rs:321` still references 5 fields. |

**Open Questions:**
- **Which `puffer-contracts` bytecode is actually deployed?** Code-level drift is now confirmed: `feat/tdx` (the checked-out branch) verifies a **5-field** preimage (no match); the matching **7-field** verifier exists only on `origin/fix/increase-guardian-signatures-security`, which in turn lacks the `ROTATE_GUARDIAN_KEY` format `feat/tdx` carries. Whether the deployed contract is 5- or 7-field decides if custody approval is broken *right now*. `(needs human input — single most important question in the catalog; cross-refs sut-analysis.md Q1.)`
- `guardian_module_address` and `chain_id` are caller-supplied — the reference encoder must compare against the real `address(this)`/`block.chainid`.

### rotate-key-preimage-matches-onchain — ROTATE_GUARDIAN_KEY preimage matches the SessionRegistry-verified message
| | |
|---|---|
| **Type** | Safety (cross-process agreement) |
| **Property** | `keccak256(abi.encode("ROTATE_GUARDIAN_KEY", moduleAddr, chainId, blockNumber, pubKey))` that the CVM agent signs verifies via `SessionRegistry.verifySessionSignature` and matches `GuardianModule`. |
| **Invariant** | `Always("rust rotate-key keccak == contract keccak")` against a reference; `Reachable("rotate-key attestation produced")`. Raw keccak, **no EIP-191 prefix**, uncompressed 65-byte pubkey — confirmed consistent with the contract's raw-message verify. |
| **Antithesis Angle** | Vary `chainId`/`blockNumber`/`address`; assert equality; guards against atakit `dev`-branch signer drift. Input-coverage + regression. |
| **Why It Matters** | A mismatch excludes a guardian from quorum → halt. |
| **Priority** | High *(LIVE: gated by `CVM_AGENT_STUB`, NOT `verify_session` — runs on the production keygen→`rotateGuardianKey` path; see [[keygen-evidence-accepted-onchain]])* |
| **Confidence** | High — `mod.rs:38-56`; matches `feat/tdx` GuardianModule.sol:335. |

**Open Questions:**
- ~~Is `_ECDSA_KEY_LENGTH == 65`?~~ **RESOLVED:** yes on `feat/tdx` (`GuardianModule.sol:37`); 65-byte uncompressed `0x04||x||y`, `ALGO_ID_ES256K = 3` matches the guardian's hardcoded `typeId:3`.
- Does the **real** DCAP CVM agent (not the atakit sim) sign `keccak256(payload)` raw, as the sim does? `(partial: the atakit sim signs raw keccak256 with no EIP-191 prefix — confirmed; the production DCAP binary is closed-source and not in any available repo — needs human input.)`

### attestation-payload-reconstruction-matches — Reconstructed validator-attestation payload matches what the agent signed
| | |
|---|---|
| **Type** | Safety |
| **Property** | The 64-byte payload `build_validator_remote_attestation_payload` reconstructs (`shared/mod.rs:179-225`) equals the payload the validator's CVM agent actually signed, so `verify_session_evidence` neither false-rejects honest evidence nor accepts a crafted mismatch. |
| **Invariant** | `Always("no silent share-vector truncation")` — guard the `.zip(shares, guardian_pks)` against unequal lengths (F2); `Always("threshold serialized as fixed 8 bytes")` — `threshold().to_be_bytes()` on a platform-dependent `usize` (W6) must be width-pinned; `Sometimes("reconstruction ran with unequal share/pubkey vector lengths")` to surface the truncation case. |
| **Antithesis Angle** | Fuzz share counts, threshold, and vector lengths; assert no silent truncation and fixed threshold width. |
| **Why It Matters** | Drift → honest custody refused (liveness); a crafted length mismatch silently hashes a different share set. |
| **Priority** | High *(on verify_session=true path; inert today)* |
| **Confidence** | High — `shared/mod.rs:179-225` (raised from Medium-High after confirming the producer is the same in-repo function). |

**Open Questions:**
- ~~Where does the validator side build the signed payload — same field order/width/pad?~~ **RESOLVED:** the producer is the *same* `build_validator_remote_attestation_payload` (`src/enclave/validator/mod.rs:96-112` calls it), so field order, threshold width, and 64-byte pad match by construction; on the x86_64 build `usize`→8 bytes on both sides. Residual risk is version skew across binaries.
- Are the share and guardian-pubkey vectors guaranteed equal-length by reef before the call? `(partial: the in-repo producer never creates a mismatch; whether reef can desync the two length fields — triggering the F2 `.zip` truncation on the verifier — needs reef-side confirmation.)`

### session-public-key-parse-no-wrong-identity — Session-key parsing never fabricates a wrong identity
| | |
|---|---|
| **Type** | Safety |
| **Property** | `parse_session_public_key` uses the `typeId=3` (ES256K) hex fallback only when the input is genuinely raw hex — never when it is a valid `PublicIdentity` JSON with a different `typeId` whose parse error was swallowed. |
| **Invariant** | `Always("session key parsed with correct typeId/curve")`; `Sometimes("JSON PublicIdentity path taken")` and `Sometimes("hex fallback path taken")` to confirm both are exercised. `Always` because a wrong curve silently changes verification semantics. |
| **Antithesis Angle** | Fuzz JSON / hex / 0x-prefixed / malformed-JSON-that-is-also-hex / varied typeId inputs. API-shape coverage. |
| **Why It Matters** | A coerced `typeId` → wrong-curve verification → custody refused, hard to diagnose (W9). |
| **Priority** | Low *(downgraded — liveness-only; on verify_session=true path)* |
| **Confidence** | High — `mod.rs:231-258` (swallowed `serde_json` error, hardcoded `typeId:3`). |

**Open Questions:**
- ~~Does SessionRegistry fail closed on a typeId/curve mismatch?~~ **RESOLVED:** yes — `SignatureVerifier.verify` reverts `UnsupportedAlgorithm` on an unknown typeId, and the fingerprint binds typeId before dispatch. So a coerced typeId can only *fail* verification (liveness), never forge a wrong-curve acceptance — W9 is **liveness-only**, not a correctness hole.
- ~~What format does the real CVM agent emit (JSON vs hex)?~~ **RESOLVED for SUT-originated payloads:** the in-repo producer emits raw `0x`-hex (`validator/mod.rs:128`), so the JSON branch is dead for SUT payloads; the real DCAP agent's exact format is still unconfirmed but cannot cause a *wrong* identity given the fail-closed verifier. `(real-agent format: needs human input, low impact.)`

### keygen-evidence-accepted-onchain — Keygen attestation evidence is accepted by `rotateGuardianKey` (LIVE on-chain) *(added in evaluation — gap G1)*
| | |
|---|---|
| **Type** | Safety (cross-process agreement) |
| **Property** | The full `KeyGenResponse { pk_hex, evidence }` the guardian returns is accepted on-chain by `GuardianModule.rotateGuardianKey()` — covering the `PublicIdentity` (`typeId==3`, 65-byte key) round-trip and the `pk_hex` uncompressed→`ecies::parse` round-trip, not just the keccak preimage. |
| **Invariant** | `Always("rotateGuardianKey accepts the guardian's keygen evidence")` against a reference encoder / anvil GuardianModule; `Always("keygen evidence PublicIdentity has typeId==3 and a 65-byte key")` (cheap shape check at the response boundary); `Sometimes("keygen carried a real (non-stub) attestation")`. |
| **Antithesis Angle** | Cross-process agreement / regression — diverse keygen inputs (block_number, chain_id, module addr); primary form is a no-chain reference encoder + shape check (R2/R8: the mock CVM agent can't catch real-agent drift). |
| **Why It Matters** | **This is the keygen twin of W0 but verified on-chain in production today** (reef `rotate_guardian_key.rs:88-195` → `rotateGuardianKey`). Drift makes a fresh guardian key un-rotatable → that guardian can't sign → correlated protocol-wide liveness loss. |
| **Priority** | High *(live on-chain path — distinct from the inert custody verify path)* |
| **Confidence** | High — `remote_attestation.rs:14-43`, `types.rs:11-32`, reef `rotate_guardian_key.rs:88-195`, contract `SignatureVerifier.sol:113-132`. |

**Open Questions:**
- Which deployed `GuardianModule` bytecode runs `rotateGuardianKey`? Shares the W0 deployment question (mechanically verifiable). `(needs human input / cast call.)`
- Does the **real** DCAP agent emit `PublicIdentity{typeId:3, 65-byte}`? `(partial: sim does; real agent closed-source — needs human input.)`

---

## Category C — Custody validation correctness

The cryptographic well-formedness gate (S1–S4) that runs on every
`validate-custody`, independent of attestation.

### stored-share-matches-committed-pubkey-share — A stored/approved share always matches its committed public-key share (S4)
| | |
|---|---|
| **Type** | Safety |
| **Property** | `verify_custody` persists and approves a share only if the decrypted share's `public_key_share()` equals `pk_set.public_key_share(i)` for the matched index. |
| **Invariant** | `Unreachable("stored a BLS share whose public-key-share ≠ the committed pk_set share")` — the cleanest encoding of a barrier that must never be crossed; plus `Sometimes("verify_custody rejected ≥1 non-matching index")` to confirm the negative path runs. This is a **genuine, currently-holdable invariant** — the strongest asset in the catalog. |
| **Antithesis Angle** | Fuzz shares: wrong index, swapped, truncated, re-encrypted, length-mismatched vectors (`.zip` F2); assert no mismatched share is ever persisted. |
| **Why It Matters** | The barrier between a forged/garbage share and "this guardian holds a bad key share." A stored non-conforming share → a validator the set can't reconstruct/exit; systematic = catastrophic. |
| **Priority** | High |
| **Confidence** | High — `mod.rs:281-310`; existing tests only assert `.is_ok()`, never this binding (it is enforced but un-asserted). |

**Open Questions:**
- W4: the share *value* is checked, but it is never bound to **this guardian's identity** — `guardian_eth_pub_keys[i]` is never compared to the enclave's own pubkey. Is that identity-pin omission by design? *(companion property [[sign-exit-index-binding]]; could escalate to an authorization gap.)*
- Could a crafted payload make an unrelated ciphertext decrypt to a share that matches a different index than the guardian's true slot? *(if yes → `Unreachable` authorization property.)*

### custody-rejects-invalid-deposit-data — Custody rejects payloads with invalid deposit signature/root/pubkey-set (S1–S3)
| | |
|---|---|
| **Type** | Safety |
| **Property** | No approval is emitted and no share persisted unless the BLS deposit signature verifies (S1), the recomputed `deposit_data_root` matches (S2), and `bls_pub_key` derives from `bls_pub_key_set` (S3). |
| **Invariant** | `Always("approval emitted ⇒ S1∧S2∧S3 passed")`; `Sometimes("rejected for invalid deposit signature")`, `Sometimes("rejected for deposit-data-root mismatch")`. `Always` for the fail-closed gate. |
| **Antithesis Angle** | Submit valid and individually-corrupted deposit payloads. **Largely unit-testable in isolation** — Antithesis value is modest except in combination with [[fork-version-source-consistency]] and faults; included as the guardrail the richer properties build on. |
| **Why It Matters** | Approving mismatched deposit data co-signs custody for a validator whose on-chain deposit doesn't match the held key. |
| **Priority** | Medium |
| **Confidence** | High — `mod.rs:260-279`, `:285-288`. |

**Open Questions:**
- See [[fork-version-source-consistency]] — a caller-chosen `fork_version` changes S1's signing domain. *(why it matters: S1 can pass for a deposit invalid under the real genesis fork version.)*

### fork-version-source-consistency — Deposit domain fork_version is the trusted one, not attacker-chosen
| | |
|---|---|
| **Type** | Safety |
| **Property** | The guardian never co-signs a custody approval for a deposit whose `fork_version` (and thus signing domain) differs from the trusted network genesis fork version. |
| **Invariant** | `Unreachable("approval emitted for a deposit built with a non-genesis fork_version")` (if a single source of truth is intended) or `Always("custody fork_version == trusted genesis")`. Today the custody/deposit path trusts the **request** `fork_version` (`types.rs:175-192`) while the BLS signing path uses the trusted `AppState` genesis — an inconsistency. |
| **Antithesis Angle** | Generate matching and mismatched `fork_version`; assert no approval on mismatch. Partly a fork-boundary / version-skew property. |
| **Why It Matters** | A wrong-but-valid-looking deposit domain; cross-component version drift across a hard fork. |
| **Priority** | Medium |
| **Confidence** | High — `types.rs:175-192` vs the AppState genesis used elsewhere. |

**Open Questions:**
- Is trusting the request `fork_version` intentional (multi-network support) or an oversight relative to the signing path? *(needs human input; decides whether this is a bug or a config surface.)*
- Who is the source of truth for `fork_version` across a fork — reef, the contract, or the guardian's genesis? *(needs human input.)*

---

## Category D — Custody approval binding & replay

### approval-binding-and-replay-resistance — The approval signature binds the fields the on-chain verifier assumes
| | |
|---|---|
| **Type** | Safety (binding) |
| **Property** | The approval ECDSA signature is a function of exactly the intended fields; it is byte-identical when only `verify_session`/session fields differ, differs when `validator_index` differs, and carries whatever replay protection the protocol requires. |
| **Invariant** | `Always("approval identical when only verify_session toggles")` (the "dark twin" — attested and unattested approvals are indistinguishable on-chain, W1); `Always("approval differs when validator_index differs")` (W3); `Reachable("approval emitted binding an attacker-chosen validator_index")`. Deferred `Unreachable` if a fix later binds session/share/guardian. |
| **Antithesis Angle** | Single-field toggling the existing tests never do; partition + the absence of an on-chain nonce contextualizes captured-approval replay (W2). |
| **Why It Matters** | The approval is a *weaker* statement than the on-chain `GuardianModule` assumes: no session/share/guardian binding, no nonce/deadline, caller-steerable `validator_index`. |
| **Priority** | High *(Medium if Q4 = internal-only and on-chain binds validator_index)* |
| **Confidence** | High — preimage `mod.rs:322-336` omits session_id/share-index/enclave-pubkey; reef sends attacker-derivable `validator_index`; only the monotonic `nextToBeProvisioned` guards on-chain. |

**Open Questions:**
- Are the missing bindings deliberately delegated to `GuardianModule` + reef? Does the contract bind `validator_index`/prevent replay? *(needs contracts review; ties to W0/Q1.)*

### approval-deterministic-and-idempotent — Identical custody requests yield byte-identical approvals across retries and guardians
| | |
|---|---|
| **Type** | Safety (idempotency / replica determinism) |
| **Property** | The same `validate-custody` request always yields the same `enclave_signature` and re-writes byte-identical share content — across retries on one guardian and across guardians given the same inputs (modulo each guardian's own enclave key). |
| **Invariant** | `Always("identical request ⇒ identical approval bytes")` (workload double-send); `AlwaysOrUnreachable("approve_custody self-recover check passes")` (the `bail!("Failed to sign correctly")` recovery, S5). Doubles as Antithesis **replay anchor**. `Always` because nondeterminism would itself be the bug. |
| **Antithesis Angle** | Crash-then-retry between share write (`mod.rs:85`) and HTTP response (reef retries → skip-provision); concurrent duplicate submits; clock jitter as a negative control (signature is time-independent — see [[guardian-routes-time-independent]]). |
| **Why It Matters** | Non-determinism → divergent on-disk state across retries or on-chain signature rejection → provisioning stalls; underpins the orphan/torn recovery story. |
| **Priority** | High *(elevated in evaluation R4 — load-bearing precondition for the assertion semantics of two Critical preimage/binding properties; the crash-retry idempotency angle is the genuinely Antithesis-fit part)* |
| **Confidence** | High — `wallet.sign_message` (ethers 2.0.14 locked), preimage is a pure function of request fields, ECIES decrypt + BLS deterministic. |

**Open Questions:**
- ~~Is ethers ECDSA RFC-6979 deterministic + canonical low-S?~~ **RESOLVED:** locked `ethers` is **2.0.14** (k256 0.13.4) → `try_sign_prehashed_rfc6979` with empty aux data ⇒ deterministic `k`; `normalize_s()` enforces low-S. The determinism `Always` holds (no polarity flip); the low-S check becomes a cheap regression sentinel.
- Does an upstream retry ever *re-encrypt* the share (non-deterministic ECIES encrypt) before re-submitting, which would change the persisted ciphertext across retries even though the guardian's output is deterministic? `(partial: the guardian only decrypts, so its output is stable; upstream re-encryption behavior unverified — low impact on the guardian's own determinism.)`

---

## Category E — Sign-exit authorization & correctness

### sign-exit-requires-authorization — Exit signatures are produced only under authorized conditions
| | |
|---|---|
| **Type** | Safety / authorization (Reachability of the gap today) |
| **Property** | `sign-exit` should mint a voluntary-exit signature share only for a validator the caller is authorized to exit — not for any caller-supplied `(bls_pub_key_set, guardian_index, validator_index)` with a stored share. |
| **Invariant** | Today: `Reachable("exit signature produced with no authorization, epoch hardcoded 0")` documents the gap; `Sometimes("exit signed using an orphaned/unapproved share")` ties to [[no-orphaned-unacknowledged-secret]]; `Always("repeated identical sign-exit ⇒ identical share")` (deterministic, infinitely replayable). An aspirational `Always(exit_signed ⇒ authorized)` once an authorization gate exists. |
| **Antithesis Angle** | Workload requests exits for shares it never legitimately provisioned; crash between custody write and approval, then sign-exit on the orphaned share. |
| **Why It Matters** | Exit shares reaching ≥M guardians **force-eject live validators** (highest-severity active harm, W10/§13d). |
| **Priority** | High *(Medium if Q4 = internal-only)* |
| **Confidence** | High — `mod.rs:352-370` (all inputs from request, epoch 0, no gate), no middleware. |

**Open Questions:**
- Is the absence of any sign-exit authorization **by design** (trusts reef over a private link)? *(needs human input — likely by-design-but-risky; flag for human judgment, do not assert as a bug.)*
- Why is the exit epoch hardcoded to 0 — is current-epoch required for the exit to be valid on the beacon chain? *(needs human input; if current-epoch is required, the exit path is broken independently — see [[sign-exit-index-binding]].)*

### sign-exit-index-binding — A share stored at custody is retrievable at sign-exit (store-index == read-index)
| | |
|---|---|
| **Type** | Safety / liveness |
| **Property** | A BLS share persisted by `validate-custody` (filename = the public-key share at the index that decrypted) must be findable by `sign-exit` for the caller-supplied `guardian_index`. |
| **Invariant** | `AlwaysOrUnreachable("sign-exit reads the share stored for this guardian's index")`; `Reachable("sign-exit 500 share-not-found while a share for that pk_set exists under a different index")`; `Sometimes("sign-exit requested with guardian_index != the index that decrypted at custody")`. |
| **Antithesis Angle** | Drive custody on a guardian whose share decrypts at index i>0, then sign-exit with `guardian_index=0` (reef's hardcoded value) → lookup miss. |
| **Why It Matters** | reef hardcodes `guardian_index=0` (`eject_validator.rs:170`); if a guardian's share decrypts at any index ≠ 0, that guardian silently cannot sign exits → the set may never reach threshold to eject a live validator (funds stuck). Novel cross-endpoint binding bug. |
| **Priority** | High |
| **Confidence** | High — store `mod.rs:82-85` (index from decrypt loop) vs read `mod.rs:355-361` (index from request); reef hardcodes 0. |

**Open Questions:**
- ~~Does reef ever send a non-zero or per-guardian `guardian_index`?~~ **RESOLVED:** reef *always* sends `guardian_index = 0` (sole call site `eject_validator.rs:170`; no per-guardian index field in `GuardianConfig`). Confirmed: any guardian whose share decrypts at index ≠ 0 is silently mute on exits.
- Is the guardian-list ordering (which fixes the decrypt index) stable across the protocol's lifetime? `(needs human input — protocol/ops; determines how often a guardian lands at index ≠ 0.)`

### unattested-share-cannot-become-exit-capability — No exit-signing capability from an unattested/orphaned share *(added in evaluation — gap G2, the "catastrophe chain")*
| | |
|---|---|
| **Type** | Safety (end-to-end timeline) |
| **Property** | A share that was persisted without verified attestation (verify_session=false) or orphaned by a crash must not be usable by `sign-exit` to mint a voluntary-exit signature. |
| **Invariant** | `Reachable("exit signature produced for a share whose custody had verify_session=false")` (witnesses the gap today); `Sometimes("sign-exit signed an orphaned share with no recorded approval")` (crash-composed; needs node-termination); aspirational `Always("sign-exit ⇒ the share's custody verification ran")`. Needs an SUT-side per-share provenance marker (MISSING). |
| **Antithesis Angle** | The single highest-value **end-to-end timeline**: (1) `validate-custody` with verify_session=false *or* crash after `write_bls_key` before approval; (2) `sign-exit` on that share; (3) assert an exit sig was produced for an unattested/unacknowledged share. Composes a request sequence with a fault — exactly Antithesis's strength. |
| **Why It Matters** | An unattested/orphaned BLS share becomes a live force-eject capability; across ≥M guardians this force-exits live, funded validators (§13d). This is the master invariant w7-6 made concrete. |
| **Priority** | High *(Critical if Q4=untrusted-reachable and Q6=not-by-design)* |
| **Confidence** | High — composes confirmed facts: `mod.rs:71` (gate), `:82-85` (persist-before-approval), `:352-394` (ungated exit). |

**Open Questions:**
- Is the ungated/unattested-share acceptance **by design** (guardian trusts reef over a private link)? `(needs human input — accepted-risk vs critical bug; cross-refs Q4/Q6.)`
- Dominance: does building this end-to-end timeline subsume the individual [[attestation-verified-before-approval]] / [[no-orphaned-unacknowledged-secret]] / [[sign-exit-requires-authorization]] rows, or are they the unit-level decomposition? `(curation — see evaluation Bias B1.)`

---

## Category F — Key persistence & crash recovery

### key-write-durable-or-rejected — A persisted key reads back intact or the read fails closed
| | |
|---|---|
| **Type** | Safety (BLS branch — confirmed) + crash-recovery liveness (ETH branch) |
| **Property** | A key written by the guardian is later read back byte-identical, or the read returns an error — never a different/corrupted key silently used to sign. |
| **Invariant** | `Unreachable("sign-exit signed with a key parsed from a truncated/empty file")` for the **BLS branch** (confirmed: `SecretKeySet::from_bytes`→`Poly::from_bytes` fails OPEN — empty or even-hex-<32-byte content → zero key, no error); `AlwaysOrUnreachable("a non-empty ETH key file never parses to a wrong key")` for the **ETH branch** (fails closed — libsecp256k1 length+range checks); + `eventually_` recovery check after faults lift. |
| **Antithesis Angle** | **Requires node-termination.** Kill/restart in the `fs::write`→(no-)flush window (`key_management.rs:9-14`, no fsync/atomic/lock), re-issue the dependent op, assert read-back integrity. |
| **Why It Matters** | The enclave ETH sk is fresh-random with no backup/replication → loss is unrecoverable; a silent wrong-key read would forge signatures. |
| **Priority** | High *(requires node-termination)* |
| **Confidence** | High. |

**Open Questions:**
- Is `guardian-data` durable across Antithesis termination, or handed a fresh FS? `(needs human input — Antithesis tenant/volume config; decides whether loss vs corruption dominates.)`
- ~~Can truncated bytes parse to a different valid key?~~ **RESOLVED (escalation):** the **BLS** read path fails OPEN to a zero key (empty *or* even-hex-<32-byte); the **ETH** path fails closed. So a truncated/empty BLS share → a successful sign-exit with a *zero* key. This is a silent wrong-key safety break, not just liveness.

### no-orphaned-unacknowledged-secret — No persisted secret without a returned approval (or cleanup)
| | |
|---|---|
| **Type** | Safety / liveness (reconciliation) |
| **Property** | Every persisted ETH key / BLS share eventually corresponds to a successful response to the caller, or is cleaned up. |
| **Invariant** | `Reachable("secret persisted before the operation succeeded")` to witness the window; `finally_`/`eventually_` reconciliation: persisted-secrets ⊆ (acknowledged ∪ cleaned) after faults stop. |
| **Antithesis Angle** | Fail/kill after the write (`mod.rs:31` keygen, `:82-85` custody) but before the response; enumerate on-disk keys vs what reef received. Crash variant **requires node-termination**; the error-after-write variant is fault-free (e.g. `approve_custody` errors on a bad `guardian_module_address.parse()`). |
| **Why It Matters** | An orphaned BLS share is a live, unauthorized exit-signing capability (ties to [[sign-exit-requires-authorization]]) and accumulates unbounded (no GC). |
| **Priority** | High |
| **Confidence** | High — write-before-success ordering confirmed; no reconciliation/GC found. |

**Open Questions:**
- Is there any reconciliation/GC anywhere (reef-side or guardian-side)? *(none found in the guardian; reef-side unknown — partial.)*
- Is an orphaned share reachable by sign-exit in the production topology? *(ties to Q4.)*

### torn-read-never-yields-wrong-key — A concurrent write+read on a key file never signs with a torn/zero key
| | |
|---|---|
| **Type** | Safety |
| **Property** | When `validate-custody` (O_TRUNC write) races `sign-exit` (read) on the same BLS share file, sign-exit either signs with the correct fully-written key or returns an error — never signs with an empty/truncated key. |
| **Invariant** | `Unreachable("sign-exit produced a signature from a torn/zero key")` + `Reachable("torn/empty read observed during concurrent write")`. `Unreachable` for the dangerous outcome. |
| **Antithesis Angle** | Concurrent same-share custody + sign-exit; **thread-pause/CPU-modulation** to land the reader inside the truncate→write window; disk-latency to widen it. |
| **Why It Matters** | **Escalated finding:** an empty file in the O_TRUNC window deserializes via `blsttc` with no error to a **zero-coefficient key** whose `secret_key()` is `Fr::zero()` → sign-exit returns **HTTP 200 with a wrong (zero-key) signature share**, not a fail-closed 500. Across guardians this corrupts threshold exit signing. |
| **Priority** | High |
| **Confidence** | High — no FS locking; empty-file→zero-key confirmed in blsttc source; filename-collision for the same share confirmed. |

**Open Questions:**
- ~~Is a partial (non-empty) read reachable and does it deserialize to a usable wrong key?~~ **RESOLVED:** confirmed — any short read (empty, or even-hex-<32-byte) of the 32-byte share file → blsttc zero key with no error (fails OPEN). Needs only the `O_TRUNC` window, robust regardless of write atomicity.
- Are custody and exit ever truly concurrent for the same validator in reef's flow? `(needs human input — decides real-world reachability vs adversarial-only; the zero-key read also reachable via crash, see [[key-write-durable-or-rejected]].)`

### missing-state-request-fails-clean — A request whose prerequisite state is missing fails cleanly, never panics/hangs
| | |
|---|---|
| **Type** | Safety |
| **Property** | validate-custody without a prior keygen, or sign-exit without a prior stored share (incl. after a restart onto a fresh volume), returns a clean HTTP error — never a panic/connection-reset, never a 200 without the key. |
| **Invariant** | `Always("missing prerequisite key ⇒ structured HTTP error, never connection reset")`; `AlwaysOrUnreachable("missing enclave key ⇒ 500, never an approval")`. |
| **Antithesis Angle** | Drive endpoints out of order, and (with node-termination) restart onto a reset volume then replay requests for old pubkeys. |
| **Why It Matters** | `/upcheck` stays 200 on a key-less guardian (false-healthy); the missing-state path must not collide with the panic vectors of [[malformed-input-never-panics]]. |
| **Priority** | Medium |
| **Confidence** | High — `fetch_eth_key` gate `mod.rs:64`, `fetch_bls_sk` gate; both `Err`→500. |

**Open Questions:**
- Should `GET /eth/v1/keygen` return `[]` rather than 500 on an empty dir? *(minor contract question.)*
- Fresh vs persisted volume on Antithesis termination? *(shared with key-write-durable.)*

---

## Category G — Liveness, availability & resource bounds

### dependency-hang-makes-progress — A stuck CVM agent or RPC never hangs a request forever
| | |
|---|---|
| **Type** | Liveness |
| **Property** | keygen / validate-custody eventually return a terminal HTTP status; a hung CVM-agent socket or SessionRegistry RPC must not pin a worker indefinitely. |
| **Invariant** | `eventually_`/`finally_` ("the op reaches 200/500 within a bounded window after the dependency recovers"); `Reachable("request still pending while dependency hung")` as evidence of the hang. |
| **Antithesis Angle** | Partition/hang the CVM-agent socket or RPC (connect-but-stall), then `ANTITHESIS_STOP_FAULTS`; assert progress. No node-termination needed. |
| **Why It Matters** | Neither dependency has an app-level or library timeout (`cvm_agent.rs`, `session_registry.rs`); a hung socket pins a tokio worker forever, and if reef also lacks a timeout, provisioning stalls indefinitely. |
| **Priority** | High |
| **Confidence** | High — confirmed no timeout in the guardian or the `automata-cvm-agent` client. |

**Open Questions:**
- ~~Does reef set a client timeout on guardian calls?~~ **RESOLVED: NO.** reef's guardian client *is* the SUT client (`ClientBuilder` → bare `reqwest::Client::new()`, no `.timeout()`), and reef wraps no `tokio::time::timeout`. So a hung CVM agent during keygen stalls reef's whole provisioning step indefinitely (unrecoverable without operator action). *(Severity caveat: the upstream puffer-ingestor→reef hop does carry a 30s timeout — see evaluation Bias B2 — so the "indefinite" stall is local to the guardian↔reef layer.)*
- Should the guardian itself wrap CVM/RPC calls in `tokio::time::timeout`? `(open — design recommendation, not yet decided.)`
- The RPC-hang variant is only live when `verify_session=true` (not prod today, P1).

### upcheck-live-under-load — /upcheck stays responsive under concurrent blocking load
| | |
|---|---|
| **Type** | Liveness |
| **Property** | The runtime services `/upcheck` even while ≥ worker-thread-count custody/keygen requests are in flight, and recovers after faults lift. |
| **Invariant** | `Sometimes("/upcheck responded fast under K concurrent blocking requests")` + `eventually_` health recovers. `Sometimes` because it is a meaningful liveness state under stress. |
| **Antithesis Angle** | Concurrent custody/keygen + **node-throttle / CPU-modulation / disk-latency**; probe `/upcheck`. The heavy work (blocking fs, ECIES decrypt loop, BLS) runs on the async workers with no `spawn_blocking`. |
| **Why It Matters** | `/upcheck` is reef's health gate and the only true liveness signal; head-of-line blocking makes a live guardian look dead → dropped from the M-of-N quorum (or hides a hung one). |
| **Priority** | High |
| **Confidence** | High — no `spawn_blocking`/`block_in_place`, no tower layers, no dependency timeouts. |

**Open Questions:**
- Production worker-thread count (CVM vCPUs) and reef's health-check timeout/debounce? `(needs human input — sets how easily starvation trips.)`
- ~~Does the ECIES trial-decrypt loop dominate latency?~~ **RESOLVED:** yes — one full `ecies::decrypt` per share, O(N), synchronous on the worker, and **N is an uncapped caller-controlled `Vec` (no body limit)**, so a single crafted `validate-custody` is arbitrarily expensive and starvation is provokable with very few requests.

### bounded-resource-usage — Key dir, FDs, and request memory stay bounded under load and failure
| | |
|---|---|
| **Type** | Safety / liveness |
| **Property** | Repeated keygen, failed custody, and failed CVM/RPC attempts do not exhaust disk inodes, file descriptors, or memory. |
| **Invariant** | `Reachable("key dir grew with no GC")` (monotonic growth documented); `Sometimes("FD count stayed bounded across repeated dependency failures")`. |
| **Antithesis Angle** | Loop keygen + failing custody under CVM/RPC faults; measure key-dir size, process FD count, RSS; send oversized bodies (no body-size limit). |
| **Why It Matters** | No GC, no body limit, per-request connections (provider/socket rebuilt each call); orphans accumulate; a large body is a memory/CPU DoS against an identical-binary quorum. |
| **Priority** | Medium |
| **Confidence** | Medium-High. |

**Open Questions:**
- ~~Do no-timeout stuck connections leak FDs or cancel cleanly?~~ **RESOLVED:** axum 0.6 does **not** cancel the handler future on client disconnect, so a hung connection holds its socket FD for the full unbounded hang; no body-size limit; key-dir has no GC/cap (`delete_*` is test-only). The numeric `ulimit -n` is the Docker default (~1024) — `(needs human input to confirm the deployed limit.)`
- Are ETH keys meant to rotate frequently (sets the growth rate)? `(needs human input — operational policy.)`

---

## Category H — Input robustness, lifecycle & representation

### malformed-input-never-panics — Malformed-length request fields never panic a handler
| | |
|---|---|
| **Type** | Safety |
| **Property** | A request with wrong-length hex fields yields a structured HTTP error, never a panic / dropped connection. |
| **Invariant** | `Unreachable("guardian handler panicked on malformed input")` (cleanest via a `CatchPanicLayer` that fires the assertion); after a fix, `Always("malformed-length field ⇒ 4xx, never a panic")` + `Sometimes("a malformed-length signature was rejected")`. |
| **Antithesis Angle** | Fuzz `signature` ≠ 96 B, `deposit_data_root` ≠ 32 B (F7/F8), odd/non/empty/oversized hex; GET list with crafted filenames (F11). F8 fires on **every** validate-custody regardless of `verify_session`. |
| **Why It Matters** | No `CatchPanicLayer` → panic = connection reset (not 500); if the endpoint is untrusted-reachable, F8 is a trivial remote DoS (one tiny request per worker). |
| **Priority** | High *(Medium if Q4 = internal-only)* |
| **Confidence** | High — `types.rs:167-173` (F8), `mod.rs:135-136` (F7), `types.rs:50` (F11); no middleware `bin/guardian.rs:35-65`. |

**Open Questions:**
- Is `validate-custody` reachable by untrusted clients (Q4)? `(needs human input — network topology; High ⇄ Medium.)`
- ~~Any other unchecked `copy_from_slice`/slice-index sites?~~ **RESOLVED:** the complete reachable-panic inventory across the four routes is exactly **F7, F8, F11**; all other length conversions are guarded (`bail!`/`?`) or use `FixedVector` (truncate/pad, no panic).

### startup-bind-failure-not-silent — Startup failure and not-ready states are not silently healthy
| | |
|---|---|
| **Type** | Safety / liveness |
| **Property** | A bind failure must not cause a silent healthy-looking exit, and `/upcheck` should not report 200 when the guardian cannot actually serve (no keys, no `SESSION_REGISTRY_*`, dead CVM socket). |
| **Invariant** | `Reachable("server bind result discarded / process exited silently")`; aspirational `Always("/upcheck 200 ⇒ guardian can serve")` (readiness vs liveness split). |
| **Antithesis Angle** | Start with a held port / missing env / dead CVM socket; probe `/upcheck`; node-termination helps reach restart-into-bad-config. |
| **Why It Matters** | `axum::Server::bind(...)` result is discarded (`bin/guardian.rs:69`) → silent exit; `/upcheck` returns 200 unconditionally (`health.rs`) → reef can route work to a guardian that can't serve. |
| **Priority** | Medium |
| **Confidence** | High. |

**Open Questions:**
- Does the deployment supervisor (systemd/k8s/atakit) restart on silent exit, masking this? *(needs human input.)*

### pubkey-representation-consistent — One key, consistent across keygen/list/disk/wire representations
| | |
|---|---|
| **Type** | Safety |
| **Property** | The uncompressed pubkey returned by keygen re-compresses to the on-disk filename and the list-endpoint value, and the base64/hex wire encodings round-trip with reef. |
| **Invariant** | `Always("returned pubkey compresses to the stored filename")`; `Reachable("validate-custody 'Could not fetch guardian enclave public key' while the eth_keys dir is non-empty")` (the confused-identity symptom). |
| **Antithesis Angle** | Assert the returned hex compresses to the saved filename; round-trip serde across reef's base64/hex split. Coverage + regression. |
| **Why It Matters** | keygen returns *uncompressed* (`types.rs:18`) but disk/list use *compressed* (`eth_keys.rs`), and the wire uses base64 — one key with three faces; any encoding drift yields a false "key missing" 500. **Latent, not live today** (see Open Questions). |
| **Priority** | Medium *(latent — assert the round-trip as a regression tripwire)* |
| **Confidence** | High. |

**Open Questions:**
- ~~Does reef store/compare the uncompressed or compressed form?~~ **RESOLVED:** reef round-trips the **uncompressed** form (config stores 65-byte `0x04…`; `parse_slice(None)` auto-detects; shares the `ecies::PublicKey` type via the git dep) and the guardian re-compresses on lookup — so the split **does not bite today**. It is a latent fragility: assert the returned-key→stored-filename round-trip as a tripwire against future encoding/version drift, rather than treating it as a live bug.
- Is keygen returning the *uncompressed* form (while disk/list use compressed) intentional, or an oversight? `(needs human input — design intent.)`
- Does any consumer ever string-match a keygen-returned key against the `GET` list output (which would expose the compressed/uncompressed split)? `(partial: reef does not today; other/future consumers unverified.)`
- Is the mixed camelCase/snake_case + base64/hex wire encoding intentional? `(needs human input — API-contract intent.)`

### trial-decrypt-distinguishes-corrupt-from-foreign — Trial decryption distinguishes a corrupt share from a not-mine share
| | |
|---|---|
| **Type** | Safety / observability |
| **Property** | The `verify_custody` decrypt loop should not conflate "ciphertext is corrupt" with "this share isn't mine" — a single matching share among malformed neighbors still succeeds, but corruption is observable. |
| **Invariant** | `Sometimes("a malformed share was skipped while custody still succeeded")`; `Always("custody success ⇒ ≥1 share matched")` as a refactor sentinel. |
| **Antithesis Angle** | Byte-flip ciphertexts at various indices; assert skip-not-abort and that a good share still wins; N junk decaps stress the blocking worker. |
| **Why It Matters** | Corrupt-vs-foreign conflation blinds operators to a malformed payload; N junk shares are a cheap DoS on a blocking decrypt loop. |
| **Priority** | Medium |
| **Confidence** | High — `mod.rs:291-309` (`continue` on error, no log), `ecies-0.2.7` decrypt fails closed. |

**Open Questions:**
- Should a corrupt (vs simply non-matching) ciphertext be logged/surfaced distinctly? *(product/observability decision.)*

### guardian-routes-time-independent — Signed outputs are independent of the wall clock
| | |
|---|---|
| **Type** | Safety (negative-space invariant) |
| **Property** | The digests the guardian signs are independent of the system clock; no guardian route reads wall-clock time. |
| **Invariant** | `Always("identical request ⇒ identical signed digest under clock jitter")` + `Unreachable("a guardian route read SystemTime/Instant")`. A regression tripwire for future time-based logic. |
| **Antithesis Angle** | Step the VM clock between identical requests (clock-jitter fault); assert byte-identical digests. |
| **Why It Matters** | Certifies clock-jitter robustness and localizes the single *ignored* time field (session `expiresAt`, F10); a tripwire for the day someone adds expiry/timeout logic. |
| **Priority** | Low |
| **Confidence** | High — epoch hardcoded 0 (`mod.rs:364`), no `tokio::time`/timeout on guardian routes. |

**Open Questions:**
- Is exit epoch 0 semantically correct, or must it be current-epoch? *(if current-epoch is required, this "looks-deterministic" property masks a broken exit path — ties to [[sign-exit-requires-authorization]]; needs human input.)*

### no-request-authentication — Privileged endpoints are reachable with no authentication (threat-model fact)
| | |
|---|---|
| **Type** | Reachability (documenting) |
| **Property** | Every privileged guardian endpoint is invokable with no authentication; the security model rests entirely on network isolation. |
| **Invariant** | `Reachable("privileged endpoint served an unauthenticated request")` — documents the threat model that gates exploitability of F1/W10/W1/W3 and the panic DoS. |
| **Antithesis Angle** | The workload *is* an unauthenticated caller; this property frames why the adversarial properties matter. |
| **Why It Matters** | If the network-isolation assumption is wrong (Q4), the whole catalog's "attacker-controlled field" properties become remotely exploitable. |
| **Priority** | Low (gating fact, not a bug per se) |
| **Confidence** | High — no tower/auth middleware on the router. |

**Open Questions:**
- Are `validate-custody`/`sign-exit` reachable by untrusted clients in the TDX topology? *(Q4 — highest-leverage question across the catalog; needs human input.)*

---

## Catalog-wide Assumptions

- "Guardian binary" = the four routes in `bin/guardian.rs`. Shared-module code
  reachable only via the validator/secure-signer routers is out of scope.
- The deployed image runs from CWD `/` (so `./data` ⇒ the `guardian-data`
  volume); if false, persistence silently relocates.
- ethers `LocalWallet` ECDSA is RFC-6979 deterministic (underpins the
  determinism/idempotency properties) — **confirmed**: locked version is 2.0.14
  (k256 0.13.4), RFC-6979 + canonical low-S. (The "2.0.8" in `Cargo.toml` is the
  semver floor.)
- The guardian only ever *decrypts* shares (never encrypts), so upstream
  non-deterministic ECIES encryption doesn't affect guardian output determinism.

## Catalog-wide Open Questions (mirror of `sut-analysis.md`; need human input)

1. Which `puffer-contracts` branch/bytecode is deployed (5-field vs 7-field custody verifier)? — gates [[custody-preimage-matches-onchain-verifier]]. **Most important.**
2. Production M-of-N threshold and guardian count — blast radius of a systematic bypass.
3. Is `verify_session` intended to be flipped on? — gates all of Category A and the verify-path of Category B.
4. Are `validate-custody`/`sign-exit` reachable by untrusted clients? — sets severity of every attacker-controlled-field property.
5. Can `CVM_AGENT_STUB=true` reach a production image? — gates [[cvm-stub-never-in-production]].
6. Is sign-exit's lack of authorization and the hardcoded epoch-0 by design? — gates [[sign-exit-requires-authorization]].

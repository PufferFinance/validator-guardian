---
sut_path: /home/fawad/puffer/projects/validator-guardian
commit: f7dbd88d99de21a6d8caba5f7de8216624ab1098
updated: 2026-06-30
external_references:
  - path: /home/fawad/puffer/projects/reef
    why: workload can reuse reef/coral call patterns and the puffersecuresigner client
  - path: /home/fawad/puffer/projects/coral
    why: validator-side keygen reference for workload construction
---

# Implementability Evaluation — `guardian` binary property catalog

Lens: **Implementability** — for each property, can it actually be CHECKED given
the deployment topology, workload constraints, and codebase? Adversarial read of
"can this be observed/constructed/faulted at all," not whether the property is
worth testing.

Method: read all 27 property evidence files, the SUT request path
(`bin/guardian.rs`, `enclave/guardian/{mod.rs,handlers/*}`, `enclave/types.rs`,
`enclave/shared/mod.rs`, `io/{cvm_agent,session_registry,key_management,remote_attestation}.rs`,
`crypto/{eth_keys,bls_keys}.rs`), the validator-side producer
(`enclave/validator/mod.rs::attest_fresh_bls_key`), the client
(`client/guardian.rs`), `Cargo.toml`/`lib.rs`, the container build
(`container/Dockerfile`, `container/guardian/*`), plus two cross-repo
investigations: the atakit CVM-agent socket protocol/simulator, and the
puffer-contracts SessionRegistry+GuardianModule deployability on `feat/tdx`.

## How to read the verdicts

- **Observable** — what the assertion needs to see: `workload` (HTTP
  request/response only), `SUT` (in-process state the workload can't see), or
  `cross-process` (an external verifier — contract/CVM-agent).
- **Buildable now?** — can the harness create the preconditions and faults?
- The catalog already tags every property "Instrumentation: MISSING" and lists
  the SDK as not-yet-a-dependency. This lens asks the *next* question: given the
  axum-0.6 handler structure and the topology, **is each instrumentation point
  actually addable, and is the scenario actually reachable?**

---

# CATALOG-WIDE FINDINGS

### CW-1. SUT-side instrumentation is feasible for ~all properties, but the panic-catch (`malformed-input-never-panics`) needs a `tower`/`tower-http` dependency the crate does not have, and the handler-core boundary is the only clean injection point.

**Concern.** The catalog leans heavily on SUT-side assertions (15+ properties say
"Instrumentation: MISSING, SUT-side required"). The good news: the SUT is a
plain library crate (`puffersecuresigner`) compiled into the `guardian` bin, all
core logic lives in ordinary functions (`verify_and_sign_custody_received`,
`verify_custody`, `approve_custody`, `sign_voluntary_exit_message`,
`build_validator_remote_attestation_payload`, `fetch_bls_sk`, `write_key`), and
the axum handlers (`validate_custody.rs`, `sign_exit.rs`,
`attest_fresh_eth_key_with_blockhash.rs`) are thin `Json(req)` → core-fn → 200/500
wrappers. Adding `antithesis-sdk` and dropping `assert_always!`/`assert_reachable!`
inline at the documented line numbers is mechanical — no refactor needed for the
data-integrity, sign-exit, persistence, determinism, or preimage properties.

The **one** instrumentation point that needs more than an inline macro is
`malformed-input-never-panics`. Its preferred encoding is a `CatchPanicLayer`
that fires `assert_unreachable!` on catch. The router today has **zero `.layer()`
calls and no `tower`/`tower-http` dependency** (`Cargo.toml`, confirmed in the
property's investigation log). So implementing the clean version requires adding
`tower-http` with the `catch-panic` feature — a real (if small) dependency
addition, not just the SDK. The alternative (no layer) is to wrap each handler
body in `std::panic::catch_unwind`, but the core fns are `async` and the panicking
sites (`copy_from_slice` in `signature()`/`verify_session_evidence`) are awaited
across `.await` points, so `catch_unwind` is awkward (the future is not
`UnwindSafe`). **Net: the panic property is buildable but is the only one whose
"clean" instrumentation form pulls in a new framework dependency and a router
change — flag it as the highest-effort instrumentation item.**

**Scope.** `malformed-input-never-panics`; secondarily the panic-reset branch of
`missing-state-request-fails-clean`.

**Evidence.** `bin/guardian.rs:35-65` (no layers); `Cargo.toml` (no
tower/tower-http); `types.rs:171` F8 `copy_from_slice` inside the awaited custody
core; axum 0.6 drops the connection on uncaught panic (property investigation log,
confirmed).

**Suggested action.** Budget for adding `tower-http = { features = ["catch-panic"] }`
plus the SDK, and a one-line `.layer(CatchPanicLayer::custom(...))` on the router.
Alternatively accept a *workload-side* proxy for this property — the workload can
observe "connection reset with no HTTP status" via the reqwest error variant and
assert `Sometimes`/`Unreachable` from the client, which needs NO SUT change but
cannot distinguish a panic-reset from a genuine network fault under fault
injection. Recommend the SUT-side layer for fidelity.

---

### CW-2. Every "verify-path" property (Category A/B verify branch) is gated behind a precondition chain that is buildable but materially more expensive than the catalog's topology implies — and the cheapest variant (mock RPC) silently makes two of these properties UNCHECKABLE for their core claim.

**Concern.** The topology offers two `session-registry` variants: anvil + real
puffer-contracts (recommended), or a mock JSON-RPC. The verify-path properties
split sharply on which one is present:

- `session-verification-fails-closed` (the fail-closed-on-RPC-error half) and
  the liveness/RPC-hang half of `dependency-hang-makes-progress`: testable with
  the **cheap mock RPC** — they only need a controllable `verifySessionSignature`
  return and an injectable error/hang. Buildable.
- `custody-preimage-matches-onchain-verifier` (W0, the #1 Critical) and the
  on-chain accept/reject half of `approval-binding-and-replay-resistance`:
  **require the anvil + real GuardianModule** variant. With only a mock RPC their
  *core* claim (does the guardian's 7-field signature actually clear the deployed
  5-field on-chain verifier?) cannot be checked — a mock that returns `true`
  proves nothing about byte-agreement. So choosing the cheap variant does not
  merely weaken these two; it removes their reason to exist. The catalog says
  this ("cannot catch on-chain preimage drift") but lists anvil as *optional*;
  this lens flags that for the catalog's own Critical property, anvil is **not
  optional**.

**The anvil variant is heavier than the topology budgets.** The contracts
investigation found SessionRegistry needs **5 registry dependencies**
(`ITeeVerifier`, `ITpmAttestation`, `ISignatureVerifier`, `IBaseImageRegistry`,
`IWorkloadRegistry`) plus an `AccessManager`, and GuardianModule needs
SessionRegistry + AccessManager — an estimated ~500-700 LoC of Solidity
deploy/mock wiring into a fresh anvil, with at least one policy entry populated.
That is feasible (a `SessionRegistryMock` already exists that bypasses TDX
verification, and `DeployGuardians.s.sol`/`DeploySessionRegistry.s.sol` exist),
but it is a substantial Foundry-harness sub-project, not the "deploy two contracts
at container start" the topology suggests.

**Important nuance the catalog under-weights:** for
`custody-preimage-matches-onchain-verifier`, you do **not** need a *live*
SessionRegistry session at all to test the custody preimage — `validateProvisionNode`
recovers the guardian's ECDSA signature against stored `enclaveAddress`es, which
is independent of session verification. So the cheapest *correct* harness for the
Critical property is: deploy GuardianModule with a known enclave address set + a
trivial AccessManager (skip the whole SessionRegistry dependency tree), register
the guardian's keygen enclave address, and submit the approval. This is far
cheaper than the full session-registration path and is the variant the catalog
should specify. Alternatively the property's *primary* form — a Rust/`alloy
sol!` reference encoder differentially compared against `approve_custody`'s bytes
— needs **no chain at all** and catches the drift deterministically; the on-chain
round-trip is the confirmation, not the only check. The catalog already proposes
the reference encoder; this lens endorses it as the *primary* implementable form
and downgrades the anvil round-trip to a confirmatory extra.

**Scope.** `custody-preimage-matches-onchain-verifier`,
`approval-binding-and-replay-resistance` (on-chain half),
`session-verification-fails-closed`, `attestation-payload-reconstruction-matches`,
`rotate-key-preimage-matches-onchain`, `attestation-verified-before-approval`
(the `verify_session=true` half).

**Evidence.** Topology "Component 2" (anvil optional); contracts investigation
(5 registry deps, SessionRegistryMock exists, 5-field `LibGuardianMessages.sol:25-34`,
`DeployGuardians.s.sol`); `GuardianModule.validateProvisionNode`
recovers against `enclaveAddress` (session-independent).

**Suggested action.** Split the on-chain properties' harness needs explicitly:
(a) the **reference-encoder** form (no chain) as the primary, always-buildable
check for both preimage properties; (b) a **minimal GuardianModule-only** anvil
deploy (skip the SessionRegistry dependency tree) for the custody on-chain
round-trip; (c) the **full SessionRegistry** anvil deploy only if the
`verify_session=true` accept-path is in scope. Mark the full session-registration
deploy as a distinct, sized work item.

---

### CW-3. The `verify_session=true` happy path needs a session whose signature the SessionRegistry accepts — and the in-repo CVM simulator generates its session key with a NON-deterministic `OsRng`, so registering "the key the harness controls" requires patching atakit. This is the single most fragile precondition in the catalog.

**Concern.** Every property on the `verify_session=true` *accept* path
(`session-verification-fails-closed` happy branch,
`attestation-payload-reconstruction-matches` on the verify side, the aspirational
`Always` guardrail of `attestation-verified-before-approval`, the
`Sometimes(verify ok)` round-trips of the two preimage-match properties) needs
the same precondition chain:

1. the mock CVM agent signs the attestation payload with a session key, AND
2. that exact session key is registered in the SessionRegistry such that
   `verifySessionSignature(sessionId, sessionKey, keccak256(payload), sig)` → true.

The atakit investigation found the reusable `SimCvmAgent` generates the session
signing key via `SigningKey::random(&mut OsRng)` **per registration** — it is not
fixed and not exported. So step (2) cannot be satisfied unless the harness either
(a) patches atakit's registration flow to use a seeded/known session key (the
investigation's explicit recommendation), or (b) reads the random session pubkey
back out of the mock at runtime and registers *that* into the SessionRegistry on
the fly before each custody call. Either is doable, but both add a coupling the
topology's "mock signs with a fixed session keypair known to the harness" glosses
over: **the in-repo simulator does not have a fixed keypair.** The topology's
assumption is achievable only with an atakit patch or a runtime pubkey-extraction
+ dynamic-registration dance.

Compounding factor: the validator-side payload producer the workload reuses
(`enclave/validator/mod.rs::attest_fresh_bls_key`, `do_remote_attestation=true`)
calls the *same* `AttestationEvidence::new` → CVM-agent socket path. So to produce
a genuine signed attestation payload the *workload itself* must also reach a CVM
agent (mock) — i.e. the mock is a dependency of both the SUT and the workload's
keygen step, and the session key it uses must be the one registered on-chain. The
session-id/payload binding must be consistent end-to-end (the workload signs the
payload, gets a session_id, and that session_id+key must be the registered one).
This is constructible but is the most error-prone wiring in the whole harness.

**Scope.** All `verify_session=true` *accept*-path properties:
`session-verification-fails-closed` (happy branch),
`attestation-payload-reconstruction-matches`,
`rotate-key-preimage-matches-onchain` (round-trip),
`custody-preimage-matches-onchain-verifier` (the `Sometimes(verify ok)` flavor only),
`attestation-verified-before-approval` (aspirational `Always`).

**Evidence.** atakit investigation (`SigningKey::random(&mut OsRng)` per
registration, session key not fixed; `SimCvmAgent` reusable with `sim` feature);
`io/remote_attestation.rs:30-43` + `io/cvm_agent.rs:32-45` (SUT calls same path);
`enclave/validator/mod.rs:96-114` (workload producer uses the same
`AttestationEvidence::new`); contracts investigation (registerSession requires the
session-key fingerprint stored; mockable via SessionRegistryMock but the key must
match).

**Suggested action.** Decide early: either (a) **patch atakit's sim** to take a
fixed session key seed (cleanest, the investigation's recommendation), and
register that pubkey's fingerprint in the SessionRegistry deploy script; or (b)
make the mock agent expose its session pubkey over a control channel and have the
workload register it dynamically before driving custody. Document that the
fail-closed / error / hang variants (which dominate the *checkable today* value)
do **not** need this — they only need a SessionRegistry that returns false/errors,
which the mock RPC gives for free. Treat the accept-path as a second-phase harness.

---

### CW-4. Six properties require node-termination, which the topology already flags as OFF-by-default — but two of them ALSO depend on whether the `/data` volume survives the kill, a tenant/volume-config fact the harness author cannot set from the catalog alone.

**Concern.** The catalog and topology correctly flag node-termination as required
for `key-write-durable-or-rejected`, `no-orphaned-unacknowledged-secret` (crash
variant), and `missing-state-request-fails-clean` (restart variant), and the
torn-read/writer-crash sub-cases. This lens confirms those are un-checkable
without the fault enabled (they pass vacuously otherwise). Two finer points:

1. **The Dockerfile shell-form ENTRYPOINT defeats clean termination.**
   `container/Dockerfile` ends with `ENTRYPOINT /usr/local/bin/${BINARY_NAME}`
   (shell form). The guardian runs as a child of `/bin/sh`, not PID 1, so it does
   not receive SIGTERM directly. The topology notes this. For *node-termination*
   (Antithesis kills the VM/container abruptly) this is less critical than for
   graceful shutdown, but the durability properties specifically want the kill to
   land mid-`fs::write`; an abrupt SIGKILL works, but if the harness ever uses a
   graceful `docker stop`, the binary never gets the signal and the window is
   missed. Must switch to exec-form `ENTRYPOINT ["/usr/local/bin/guardian"]` for
   reliable termination behavior. **This is a SUT/image change the catalog assumes
   but does not own.**

2. **Volume durability across termination is an unknowable knob.**
   `key-write-durable-or-rejected` (torn file on the *same* volume) vs
   `missing-state-request-fails-clean` (fresh empty volume) are *different*
   properties that depend on whether Antithesis hands back the persisted
   `guardian-data` volume or a fresh FS after a kill. The catalog lists this as an
   open question; for implementability it means the harness author cannot
   guarantee *which* of the two properties a given termination actually exercises
   until the tenant/volume config is pinned. Both should be configured
   deliberately (durable named volume for the torn-file property; an
   explicitly-reset mount for the fresh-volume property), which may require two
   distinct test-template configs rather than one.

**Scope.** `key-write-durable-or-rejected`, `no-orphaned-unacknowledged-secret`
(crash variant), `missing-state-request-fails-clean` (restart variant),
`torn-read-never-yields-wrong-key` (writer×crash sub-case),
`approval-deterministic-and-idempotent` (crash-then-retry angle).

**Evidence.** `container/Dockerfile` shell-form ENTRYPOINT;
`container/guardian/docker-compose.yml` (`guardian-data:/data` named volume);
property files' "Fault dependency: REQUIRES node-termination — DISABLED by default"
sections; topology "Persistence & termination knob" + "Known image issue".

**Suggested action.** (1) Change the Dockerfile to exec-form ENTRYPOINT as a
prerequisite for the Category-F harness. (2) Author two volume configs (durable vs
reset) and map each F-property to the one it needs; do not assume a single
termination config covers both. (3) Confirm node-termination is enabled in the
tenant before counting any F-property as non-vacuous.

---

### CW-5. The torn-read / zero-key safety escalation is reachable WITHOUT node-termination (just the O_TRUNC window) — but reliably landing a reader inside that ~microsecond window needs Antithesis thread-pause/CPU-mod, and the SUT runs all blocking work on async workers with no spawn_blocking, which both enables the race AND makes it schedule-dependent.

**Concern.** `torn-read-never-yields-wrong-key` (and its zero-key safety claim) is
the strongest *fault-light* property: it needs only the O_TRUNC-before-write
window on a concurrent custody-write vs sign-exit on the same BLS file — no
node-termination. That is genuinely buildable: the workload drives concurrent
`validate-custody` (same share) + `sign-exit` (same guardian_index), and the
SUT-side assertion (recompute `sk.public_keys().public_key_share(i)` after
`fetch_bls_sk` and compare to the requested share pubkey) catches the zero-key
read. The instrumentation point is a clean inline assertion in
`sign_voluntary_exit_message` after `mod.rs:361` — addable.

The implementability caveat is *timing reachability*: the window is the gap
between `open(O_TRUNC)` and `write()` completing for ~64 hex bytes — microseconds.
Hitting it needs Antithesis thread-pause / CPU-modulation / io-latency to pause
the writer between truncate and write. This is exactly Antithesis's strength, so
it is feasible, but it is **not** reachable by workload concurrency alone on a
fast tmpfs — the property is only meaningfully checkable with the scheduling/IO
faults enabled, and its *Reachable* witness ("torn/empty read observed") may be
rare even then. Mark as buildable-but-fault-sensitive: the `Unreachable(wrong-key
signed)` safety assertion is cheap and always-on once instrumented (it fires if
the bug ever manifests), but the companion `Reachable(torn read observed)` that
proves the race was exercised depends on the scheduler actually landing the
interleaving.

**Scope.** `torn-read-never-yields-wrong-key`, `upcheck-live-under-load`
(same blocking-on-worker mechanism).

**Evidence.** `key_management.rs:9-14` (`fs::write` = O_TRUNC, no lock);
`mod.rs:82-85` write vs `mod.rs:356-361` read (filename collision when
guardian_index == decrypt index); no `spawn_blocking` (sut-analysis §4);
property investigation log (empty→zero-key confirmed in blsttc).

**Suggested action.** Enable thread-pause/CPU-modulation/io-latency faults for the
concurrency properties; instrument the cheap always-on safety assertion regardless
(it has value even if the race is rarely hit); treat the `Reachable(torn observed)`
as a coverage signal, not a gate.

---

### CW-6. The workload CAN construct the custody/keygen/sign-exit preconditions by reusing `puffersecuresigner` as a library — the validator-side keygen, ECIES share-encryption, deposit construction, and attestation-payload build all already exist as callable functions. This is the catalog's strongest implementability asset.

**Concern (positive).** The topology's claim that the workload can play the
validator+reef side by depending on `puffersecuresigner` is **confirmed and
strong**. `enclave/validator/mod.rs::attest_fresh_bls_key` does the entire
validator-side flow in one function: `new_bls_key(threshold-1)` →
`distribute_key_shares` → `RecipientKeys::encrypt_to_recipient` (ECIES to guardian
pubkeys) → `sign_full_deposit` → `build_validator_remote_attestation_payload` →
optional `AttestationEvidence::new`, returning a ready `BlsKeygenPayload`. The
client side (`client/guardian.rs`) already wraps all four endpoints. So for the
**non-attestation** properties (Category C/D/E/F/G/H, plus the `verify_session=false`
documenting forms of A), the workload needs no bespoke crypto — it calls library
functions and the existing `GuardianClient`. The catalog's "reuse
puffersecuresigner" assumption holds without caveat for the dominant set of
properties.

The only assembly the workload must do itself: drive `POST /eth/v1/keygen` first
to get the guardian's enclave pubkey (uncompressed hex), feed that as
`guardian_enclave_public_key` into the custody request, and (for index-binding
tests) arrange the guardian's enclave key to land at a non-zero decrypt index by
ordering `guardian_eth_pub_keys`. All of this is request-shaping the workload
controls.

**Scope.** Catalog-wide enabler; directly supports `stored-share-matches-committed-pubkey-share`,
`custody-rejects-invalid-deposit-data`, `fork-version-source-consistency`,
`sign-exit-index-binding`, `approval-deterministic-and-idempotent`,
`trial-decrypt-distinguishes-corrupt-from-foreign`, `pubkey-representation-consistent`,
`malformed-input-never-panics`.

**Evidence.** `enclave/validator/mod.rs:51-136` (`attest_fresh_bls_key` full
flow); `client/guardian.rs` (all four endpoint wrappers, public);
`lib.rs` (crate exposes `enclave`, `client`, `crypto` modules); `Cargo.toml`
(it is a normal lib crate).

**Suggested action.** Build the workload as a binary depending on
`puffersecuresigner` (path/git dep) + `antithesis-sdk`; reuse `attest_fresh_bls_key`
and `GuardianClient` directly. No precondition-construction blocker for the
non-attestation majority.

---

### CW-7. `cvm-stub-never-in-production` and `startup-bind-failure-not-silent` are config/startup properties whose CHECKABLE form depends on a "production-mode" signal the binary does not have — they can only be observed as `Reachable` today, not as the `Unreachable`/`Always` the catalog aspires to.

**Concern.** Two properties assert a guard that does not exist in the code:

- `cvm-stub-never-in-production`: the aspirational `Unreachable("production keygen
  returned empty attestation")` needs the binary to *know* it is in production.
  `AttestationEvidence::new` only reads `CVM_AGENT_STUB`; there is no prod/non-prod
  flag anywhere. So the only *implementable* assertion today is the honest
  `Reachable("keygen returned empty stub attestation when CVM_AGENT_STUB=true")` —
  a config-toggle the workload sets. The `Unreachable` form is not buildable
  without first adding a production signal to the SUT (a code change the property
  recommends but the catalog cannot assume).
- `startup-bind-failure-not-silent`: the `Reachable("server bind result discarded /
  process exited silently")` is observable (start with a held port, observe the
  process exit + `/upcheck` unreachable). But the aspirational `Always("/upcheck
  200 ⇒ guardian can serve")` requires a readiness check the binary lacks
  (`/upcheck` is unconditional 200, `health.rs:3-5`). So again only the documenting
  `Reachable` is buildable now; the `Always` needs a SUT readiness-endpoint change.

Both are buildable in their *Reachable/documenting* form (config-driven, no
exotic faults), so they are not blocked — but the lens flags that their stronger
forms are gated on SUT features that don't exist, so the catalog should not expect
the `Always`/`Unreachable` variants to be checkable against the current binary.

**Scope.** `cvm-stub-never-in-production`, `startup-bind-failure-not-silent`.

**Evidence.** `remote_attestation.rs:30-34` (only reads `CVM_AGENT_STUB`, no prod
flag); `bin/guardian.rs:69` (`_ = axum::Server::bind(...)` result discarded);
`shared/handlers/health.rs:3-5` (unconditional 200).

**Suggested action.** Implement the `Reachable`/documenting forms now (cheap,
config-driven). Mark the `Unreachable`/`Always` forms as "blocked on SUT feature"
(a production flag; a readiness endpoint) rather than instrumentation-only.

---

# PER-PROPERTY VERDICTS (condensed)

Legend: **OK** = buildable as specified; **OK\*** = buildable but with a flagged
caveat above; **BLOCKED-FORM** = the documenting form is buildable, a stronger
aspirational form is not.

| Property | Observable | Buildable now? | Notes |
|---|---|---|---|
| attestation-verified-before-approval | SUT (verified-flag tag) + workload | OK\* | `Reachable(verify_session=false)` is trivially buildable (workload toggles the bool, SUT marks the branch at `mod.rs:71`). The master `Always(exit_signed ⇒ share_was_verified)` lifecycle invariant needs a **per-share metadata bit written at custody time and read at sign-exit** — the SUT writes only the raw share bytes to disk (`mod.rs:82-85`), so this needs a sidecar metadata file or an in-process map keyed by share pubkey. Addable but is the most involved SUT instrumentation (cross-endpoint state). CW-3 for the `Always` guardrail. |
| session-verification-fails-closed | SUT + cross-process (RPC) | OK\* | Fail-closed-on-error: cheap mock RPC + network fault. Happy/accept branch + isActive/expiresAt gap: needs the session-accept precondition (CW-3). Inert unless `verify_session=true` set by workload. |
| cvm-stub-never-in-production | SUT marker | BLOCKED-FORM | `Reachable` buildable via env toggle; `Unreachable(prod)` blocked (no prod flag). CW-7. |
| custody-preimage-matches-onchain-verifier | SUT + cross-process (contract) | OK\* | Reference-encoder form: no chain, fully buildable, catches W0 deterministically. On-chain round-trip: needs minimal GuardianModule anvil deploy (CW-2). Highest-value, and the reference-encoder primary form is cheap. |
| rotate-key-preimage-matches-onchain | SUT + cross-process | OK\* | Reference-encoder form buildable now. Round-trip needs CVM mock signing + SessionRegistry accept (CW-3). On keygen-attestation path; inert if attestation off. |
| attestation-payload-reconstruction-matches | SUT | OK | Builder is in-repo and shared producer/verifier; the length-equality (`.zip` F2) and threshold-width (==8) assertions are pure inline `assert_always!` in `shared/mod.rs:200-212`. Differential test needs no chain. Only the on-chain *consume* is verify-path-gated. Strongest of the preimage cluster. |
| session-public-key-parse-no-wrong-identity | SUT | OK | Pure input-fuzz of `parse_session_public_key` (`mod.rs:231-258`); `Sometimes(json path)`/`Sometimes(hex path)` inline markers. Verify-path-gated for the end effect, but the parse function itself is callable/assertable directly. Low priority per catalog. |
| stored-share-matches-committed-pubkey-share | SUT | OK | The S4 binding assertion (`Unreachable(stored share pk ≠ pk_set.public_key_share(i))`) is a one-line inline at `mod.rs:302-305`/after `:85`. Workload builds shares via reused keygen (CW-6) and fuzzes index/swap/truncate. The strongest holdable invariant; fully buildable, no faults, no chain. |
| custody-rejects-invalid-deposit-data | workload + SUT | OK | Workload sends valid + individually-corrupted deposit payloads (reuses producer, then mutates); `Always(approval ⇒ S1∧S2∧S3)` + `Sometimes(rejected ...)` inline at `mod.rs:260-279`. Largely unit-shaped. |
| fork-version-source-consistency | workload + SUT | OK | Workload sets request `fork_version` matching/mismatching genesis; `Unreachable(approval for non-genesis fork)` inline. Buildable; the *interpretation* (bug vs feature) is human-input but the check is mechanical. |
| approval-binding-and-replay-resistance | workload (toggle) + cross-process (on-chain) | OK\* | The dark-twin (`Always(identical when only verify_session toggles)`) and `Always(differs when validator_index differs)` are **workload-only** byte comparisons — fully buildable, no SUT change. The on-chain replay/accept half needs the GuardianModule anvil deploy (CW-2). |
| approval-deterministic-and-idempotent | workload + SUT | OK | Double-send byte comparison is workload-only. The low-S/self-recover `AlwaysOrUnreachable` mirrors the existing `bail!` at `mod.rs:345-347` as an inline assertion. Crash-then-retry angle needs node-termination (CW-4) but the core determinism check does not. |
| sign-exit-requires-authorization | SUT + workload | OK | `Reachable(exit signed with no authorization, epoch 0)` is observable from the workload (call sign-exit on a stored share with arbitrary fields, get 200). The lifecycle tie to orphaned shares needs the share-metadata tag (see attestation-verified row). Buildable; severity is human-input, not implementability-blocked. |
| sign-exit-index-binding | SUT | OK\* | Needs SUT to **record the custody decrypt-index** (a sidecar/log event at `mod.rs:82`) so sign-exit can assert store-index == read-index — the index is not persisted today. Addable. Workload arranges a non-zero decrypt index by ordering `guardian_eth_pub_keys` and reusing the keygen producer (CW-6), then calls sign-exit with `guardian_index=0`. The `Reachable(500 while share exists under another index)` is workload-observable without the sidecar. |
| key-write-durable-or-rejected | SUT | OK\* | BLS-branch zero-key assertion is inline in `fetch_bls_sk`/sign-exit. **Requires node-termination** + volume-durability config (CW-4). Without the fault, only the static "writes are non-atomic" stands; passes vacuously. |
| no-orphaned-unacknowledged-secret | SUT (counters) + workload (disk enumerate) | OK\* | The error-after-write variant is **fault-free** (make `approve_custody` fail via bad `guardian_module_address.parse()` at `mod.rs:323`, after the `:85` write) — buildable now, no termination. The crash variant needs node-termination (CW-4). Reconciliation invariant needs SUT counters (persisted vs acknowledged) — inline events addable. |
| torn-read-never-yields-wrong-key | SUT | OK\* | No node-termination needed (O_TRUNC window). Inline zero-key guard in sign-exit. Reliable interleaving needs thread-pause/CPU-mod/io-latency (CW-5). |
| missing-state-request-fails-clean | workload + SUT | OK\* | Out-of-order driving is workload-only (call custody before keygen → 500). The fresh-volume-after-restart variant needs node-termination + reset volume (CW-4). The panic-vs-clean distinction shares the CatchPanicLayer concern (CW-1). |
| dependency-hang-makes-progress | workload (terminal-status) + mock control | OK\* | CVM-hang variant: mock agent `hang` mode + observe no terminal status — buildable (mock is a controllable in-container helper, atakit sim reusable, CW-3). RPC-hang variant needs `verify_session=true` + mock RPC hang (network fault on the guardian↔RPC TCP link). Liveness via `ANTITHESIS_STOP_FAULTS`. No node-termination. |
| upcheck-live-under-load | workload (probe latency) | OK | Mostly **workload-observable** (probe `/upcheck` under concurrent large-N custody load); no SUT change needed for the core `Sometimes`/recovery assertions. Amplified by node-throttle/CPU-mod/io-latency. The uncapped caller-controlled N makes starvation easy to provoke. Buildable. |
| bounded-resource-usage | workload (enumerate) + host metrics | OK\* | Key-dir growth and oversized-body memory are workload-drivable (loop keygen/failed custody, send big bodies). FD-count/RSS measurement needs **host-level observation** of the SUT container, which Antithesis can provide but is not an SDK assertion — partly out-of-SDK. The `Reachable(key dir grew, no GC)` is workload-observable via `GET /eth/v1/keygen` count. |
| malformed-input-never-panics | SUT (catch layer) or workload (reset detection) | OK\* | Highest-effort instrumentation: clean form needs `tower-http` catch-panic dep + router layer (CW-1). Workload-only fallback: observe connection-reset (reqwest error) and assert — but can't distinguish panic from network fault under faults. Input fuzzing itself is trivial. |
| startup-bind-failure-not-silent | workload (process/health) | BLOCKED-FORM | `Reachable(silent exit)` buildable (held port). `Always(/upcheck ⇒ can serve)` blocked (no readiness endpoint). CW-7. |
| pubkey-representation-consistent | workload + SUT | OK | Round-trip assertion (returned uncompressed pubkey re-compresses to stored filename / list value) is workload-observable (keygen → GET list → compare) plus an inline SUT marker. Latent-not-live per catalog; buildable as a tripwire. No faults. |
| trial-decrypt-distinguishes-corrupt-from-foreign | SUT + workload | OK | `Sometimes(malformed share skipped while custody succeeded)` + `Always(success ⇒ ≥1 matched)` inline at the decrypt loop `mod.rs:291-309`. Workload byte-flips ciphertexts (reuses producer). No faults, no chain. |
| guardian-routes-time-independent | workload (byte compare) + SUT (negative) | OK\* | `Always(identical request ⇒ identical digest under clock jitter)` is workload-observable (send identical requests, step the VM clock between, compare signature bytes) — needs the **clock-jitter fault** as a negative control (topology lists it). The `Unreachable(route read SystemTime/Instant)` is a static/code property — not really runtime-assertable without instrumenting every time call; treat as a code-review tripwire. Low priority. |
| no-request-authentication | workload | OK | The workload **is** the unauthenticated caller; `Reachable(privileged endpoint served unauthenticated)` is true by construction the moment any request succeeds. Trivially buildable; gating fact, not a bug. |

---

# PASSES (properties whose implementability is clean — no topology/instrumentation blocker)

- **stored-share-matches-committed-pubkey-share** — inline one-line assertion at a
  documented success branch; workload builds inputs via the reused keygen producer;
  no faults, no chain, no cross-process verifier. The catalog's "strongest holdable
  invariant" is also among the most cleanly implementable.
- **custody-rejects-invalid-deposit-data**, **trial-decrypt-distinguishes-corrupt-from-foreign**,
  **fork-version-source-consistency** — same shape: workload mutates a
  producer-built payload, SUT asserts the existing gate inline. No exotic deps.
- **approval-deterministic-and-idempotent** (core determinism) and
  **approval-binding-and-replay-resistance** (the verify_session-toggle and
  validator_index byte comparisons) — **workload-only**, no SUT change at all.
- **attestation-payload-reconstruction-matches** (the length/threshold-width
  assertions and differential test) — in-repo shared builder, no chain.
- **upcheck-live-under-load** — workload-observable `/upcheck` probing; the
  uncapped caller-controlled share-vector makes starvation easy to provoke.
- **pubkey-representation-consistent**, **no-request-authentication** —
  workload-observable round-trip / by-construction.
- **dependency-hang-makes-progress** (CVM-hang variant) — the mock agent is a
  controllable in-container helper and the atakit `SimCvmAgent` is reusable;
  terminal-status is workload-observable.

# UNCERTAINTIES (need confirmation before the property can be counted checkable)

1. **Is node-termination enabled on the target tenant, and does it preserve or
   reset `/data`?** (CW-4) Gates all six Category-F crash/restart properties and
   determines *which* of key-write-durable vs missing-state a kill exercises.
   Tenant/volume config — outside the catalog.
2. **Which `puffer-contracts` bytecode is deployed (5-field vs 7-field)?** Gates
   whether `custody-preimage-matches-onchain-verifier`'s on-chain round-trip
   should expect accept or reject — the *property* is implementable either way
   (the reference encoder catches the drift regardless), but the on-chain
   confirmation's expected outcome depends on this human-input fact.
3. **Will atakit's sim be patched for a fixed/known session key, or will the
   harness register the random pubkey dynamically?** (CW-3) Gates every
   `verify_session=true` *accept*-path property. Both routes work; the choice
   determines harness complexity and must be made before the accept-path is
   buildable.
4. **Is the anvil + full-SessionRegistry deploy in first-harness scope, or only
   the minimal GuardianModule deploy + mock RPC?** (CW-2) Determines whether the
   accept-path preimage round-trips are checkable in phase one. The Critical
   property's *primary* form (reference encoder) does not depend on this.
5. **Will the Dockerfile be switched to exec-form ENTRYPOINT?** (CW-4) Affects the
   reliability of termination-window timing for the durability properties; without
   it, graceful-stop-based termination misses the window.
6. **Will `tower-http` (catch-panic) be added?** (CW-1) Determines whether
   `malformed-input-never-panics` gets its high-fidelity SUT form or only the
   lower-fidelity workload-side connection-reset proxy.
7. **For `bounded-resource-usage`, can the harness observe FD count / RSS of the
   SUT container?** That half is host-metric, not an SDK assertion; the key-dir
   growth half is workload-observable. Confirm the tenant exposes process metrics.

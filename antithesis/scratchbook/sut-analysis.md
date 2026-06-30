---
sut_path: /home/fawad/puffer/projects/validator-guardian
commit: f7dbd88d99de21a6d8caba5f7de8216624ab1098
updated: 2026-06-30
external_references:
  - path: /home/fawad/puffer/projects/reef
    why: reef-guardian is the sole real consumer of the guardian via GuardianClientTrait; reveals which request fields are populated (notably verify_session=false, empty session/workload fields) and the M-of-N threshold orchestration.
  - path: /home/fawad/puffer/projects/coral
    why: coral-cli drives the validator-side BLS keygen/custody flow; confirmed coral never calls the guardian directly (it talks to the validator binary), which reframes the trust boundary.
---

# SUT Analysis — `guardian` binary

> Scope: the **`guardian`** binary of the `puffersecuresigner` crate only (not
> `validator` / `secure-signer`). Priority risk area requested by the user:
> **attestation / signature verification**. Synthesized from a 6-lens discovery
> ensemble (architecture/deps, state/concurrency, safety/liveness,
> failure-modes/assumptions, bug-history/tests/product, wildcard). Per-lens raw
> findings are in `scratchpad/sut-findings-{1..6}.md`.

## 1. What the guardian is, in one paragraph

The guardian is a remote-signing / key-management enclave server intended to run
inside a **GCP TDX Confidential VM**. It is one of an **M-of-N set of guardians**
that collectively hold a threshold BLS key for each Puffer validator. Its job is
to (a) generate an **attested enclave ETH key**, (b) **validate custody** of a
BLS key share encrypted to that ETH key — optionally verifying the validator
enclave's CVM session evidence on-chain — and then **co-sign a custody approval**
that the protocol uses to provision/activate the validator, and (c) **sign
voluntary-exit messages** with its stored BLS share so a validator can be ejected.
Its co-signature is consumed on-chain by `GuardianModule`; its caller is the
off-chain **reef-guardian** service (plus a BFF orchestrator). A single guardian
misbehaving is tolerated below threshold; a *systematic* flaw replicated across
≥ M guardians (they all run the same binary) is **catastrophic** — it can forge
validator provisioning or force-eject live validators.

## 2. Architecture & data flow

- **Process model.** Single process, `#[tokio::main]` multi-threaded runtime,
  `axum 0.6.20`. Binds `0.0.0.0:PORT` where `PORT` = `GUARDIAN_PORT` env > argv[1]
  > `3031`. The container/deployment uses `9001` via the env file. **The bind
  result is discarded** (`bin/guardian.rs:69` `_ = axum::Server::bind(...)...`),
  so a bind failure causes a **silent exit**, not a crash/log.
- **Middleware: none.** No auth, no TLS, no `TimeoutLayer`, no body-size limit,
  no `CatchPanicLayer`, no tower layers at all (`bin/guardian.rs:35-65`).
  Consequence: a panic in any handler **drops the TCP connection with no HTTP
  response** (process survives; caller sees a connection reset, not a 500).
- **Shared state:** `AppState { genesis_fork_version }` — immutable, set once in
  `main`. No other in-process mutable state.
- **Routes & request paths:**

| Method + Path | Handler → core fn | What it does | TDX/attestation |
|---|---|---|---|
| `GET /upcheck` | `shared::handlers::health` | Unconditional 200 | none — only true process-liveness signal |
| `POST /eth/v1/keygen` | `guardian::attest_new_eth_key_with_blockhash` | Generate secp256k1 ETH key, **persist sk**, build `keccak256(abi.encode("ROTATE_GUARDIAN_KEY", moduleAddr, chainId, blockNumber, pubKey))`, get CVM-agent session signature over it | **produces** attestation |
| `GET /eth/v1/keygen` | `shared::handlers::list_eth_keys` | List eth key filenames | none |
| `POST /guardian/v1/validate-custody` | `guardian::verify_and_sign_custody_received` | fetch guardian ETH sk → *(optional)* session-evidence verify → deposit-msg verify → ECIES-decrypt BLS share → **persist share** → EIP-191 sign approval | **verifies** attestation *(only if `verify_session=true`)* |
| `POST /guardian/v1/sign-exit` | `guardian::sign_voluntary_exit_message` | fetch saved BLS share → sign `VoluntaryExit` at **hardcoded epoch 0** | none |

- **Serialization boundary** (`enclave/types.rs`): JSON. Mixed conventions —
  `ValidateCustodyRequest`/responses are `camelCase`; `BlsKeygenPayload` fields
  are snake_case; `guardian_enclave_public_key` serializes as **base64**
  (libsecp256k1's human-readable impl) while every other key/sig is **hex**. A
  client serialization mismatch silently fails to deserialize → 422/400.
- **Key lifecycle & instance affinity.** keygen *produces* the enclave ETH key
  (filename = **compressed** 33-byte pubkey hex); validate-custody *consumes* it
  by re-compressing the **uncompressed** pubkey the caller supplies
  (`mod.rs:64`, note `KeyGenResponse::from_eth_key` returns the **uncompressed**
  form — a footgun: the value clients receive ≠ the on-disk filename).
  validate-custody therefore **requires a prior keygen on the same disk**, and
  sign-exit **requires a prior validate-custody**. There is **no replication** —
  state is local to one VM/volume.

## 3. State management & persistence

- **Single source of truth = disk.** No in-memory key cache; every operation
  re-reads from disk.
- **Location:** relative `./data/keys/eth_keys/` and `./data/keys/bls_keys/`
  (`constants.rs:1-4`). Resolves to `/data/...` **only** because the runtime
  Docker stage sets no `WORKDIR` (CWD = `/`) and that matches the
  `guardian-data:/data` volume (`atakit.json` provisions an encrypted 10GB
  `guardian-data` disk). Running from any other CWD writes keys elsewhere.
- **Format:** secret keys stored as **plaintext lowercase hex**, no `0x`, no
  trailing newline, **no metadata, no integrity check** (`key_management.rs`).
  Secrecy depends entirely on **GCP disk-level encryption**, not application
  encryption. (An encrypted-keystore writer `write_bls_keystore` exists but the
  custody path deliberately uses plaintext `write_bls_key`.)
- **Durability gaps (confirmed: zero `fsync`/`sync_all`/`rename`/`tempfile` in
  non-test code):** `write_key` = `fs::write` → open+`O_TRUNC`+write, **never
  fsync'd** (neither file nor parent dir), **not atomic**, **no lock**, and
  unconditional overwrite. A VM reset / power loss (Antithesis-injectable) can
  silently lose a just-"saved" key or leave a **zero-length / truncated** file.
  Loss of the enclave ETH sk is unrecoverable (each keygen is a fresh random
  key). A later read hex-decodes whatever bytes survive — realistically **fails
  closed** (downstream length checks reject it → 500) but corruption is silent
  and permanent with no backup.
- **Write-before-success ordering (state corruption window):** keygen persists
  the ETH sk **before** awaiting CVM-agent attestation (`mod.rs:31` then `:56`);
  validate-custody persists the decrypted BLS share **before** `approve_custody`
  signs (`mod.rs:82-85`). Any error/panic/crash after the write but before the
  response leaves an **orphaned secret on disk that the caller never learns
  about**, and reef may skip-provision — no rollback, no reconciliation,
  accumulates across retries.

## 4. Concurrency model

- Multi-threaded tokio, but the **heavy work inside handlers is synchronous and
  blocking** and runs **directly on the async workers** — blocking `fs::read/
  write`, the ECIES `verify_custody` decrypt loop (`mod.rs:291`), and BLS
  ops. **No `spawn_blocking` / `block_in_place` anywhere** (grep = 0). Under
  concurrent custody/exit load on a slow encrypted disk this causes
  **head-of-line blocking** that can stall all workers, including `/upcheck` —
  the most realistic availability fault.
- **No locking around the filesystem** (only the test mock has a `Mutex`).
  - **Torn read on same BLS file:** two `validate-custody` for the same share
    both `write_bls_key` to the same path (O_TRUNC) with no lock; a concurrent
    `sign-exit` reader can observe a transient empty/torn file → intermittent
    500. Content is deterministic so no *wrong-key* outcome — fails closed but
    flaky.
  - **Ruled out:** two concurrent `POST /eth/v1/keygen` (distinct random
    pubkeys → distinct files); validate-custody vs `GET` list (different dirs);
    `read_dir` during a write (benign). No check-then-act TOCTOU on guardian
    routes (writes don't gate on `*_exists`; `delete_*` is unreachable from any
    guardian route).
- Guardian routes do **not** touch the slash-protection DB (that is the
  validator/secure-signer signing path) — confirmed; do not spend test budget
  there for the guardian.

## 5. External dependencies & integration points

| Dependency | Where | Reuse | Timeout | Retry | On failure |
|---|---|---|---|---|---|
| **CVM Agent** (Unix socket `/app/cvm-agent.sock`), crate `automata-cvm-agent` | `io/cvm_agent.rs`, `io/remote_attestation.rs` | `CvmAgent::new` **per request**, no pool | **none** (atakit `CvmAgent::post` has no timeout on connect/handshake/send/read) | **none** | connect/handshake/non-2xx → error → **500**. `CVM_AGENT_STUB=true` short-circuits to **empty default evidence returned as 201** |
| **SessionRegistry** contract via alloy `eth_call` | `io/session_registry.rs` | **provider rebuilt per call**, in *both* `verify_session_signature` and `get_session` → up to **2 RPC round-trips** per workload-checked request | none (alloy default) | none | any error → `.context(...)?` → **500 (fail-closed; never fail-open)** |
| **On-disk key dir** | `io/key_management.rs` | — | — | — | FS error → 500; no atomic write/lock/fsync |
| **Env vars** | `GUARDIAN_PORT`,`GENESIS_FORK_VERSION` (startup `.expect` → panic-exit), `CVM_AGENT_STUB`,`CVM_AGENT_SOCKET_PATH` (per keygen), `SESSION_REGISTRY_RPC_URL`,`SESSION_REGISTRY_ADDRESS` (lazy per verifying request → 500 if missing) | — | — | — | committed `env` only sets `RUST_LOG`+`GUARDIAN_PORT`; **no `SESSION_REGISTRY_*` and no stub var set** by default |

**Trust-boundary reframe (wildcard + property discovery):** *coral never calls
the guardian.* The custody fan-out and `provision_node` submission are driven by
an off-chain **reef/BFF orchestrator that the guardian does not authenticate** —
but **signature aggregation is on-chain**: each guardian independently validates,
signs, and submits its *own* `provision_node([single_sig], …)`, and
`GuardianModule`/`PufferProtocol` is the threshold aggregator/enforcer (P4 in the
property catalog). That backend + the `puffer-contracts` on-chain verifier are the
real trust boundary. The guardian implicitly trusts every field in the request.

## 6. Claimed guarantees vs. enforced behavior

**Enforced safety invariants** (all in `enclave/guardian/mod.rs`, strong seeds
for `Always`/`Unreachable` properties):

- **S1** DepositMessage BLS signature must verify against `pk_set.public_key()`, else `bail!("DepositMessage signature invalid")` (`:263-269`).
- **S2** Recomputed `deposit_data_root` must equal the supplied one (`:271-277`).
- **S3** `bls_pub_key` must be derivable from `bls_pub_key_set` (`:285-288`).
- **S4** A share is accepted/stored only if it decrypts under the enclave key **and** its public-key-share equals `pk_set.public_key_share(i)` (`:291-309`). No match ⇒ no signature, no write.
- **S5** The custody approval signature must recover to the enclave's **own** wallet address, else `bail!("Failed to sign correctly")` (`:345-347`).
- **S6** The approval commits, in fixed ABI order, to `(guardianModuleAddress, chainId, validatorIndex, blsPubKey, withdrawalCredentials, depositSignature, depositDataRoot)` (`:322-336`).
- **S7** keygen commits the exact contract preimage `keccak256(abi.encode("ROTATE_GUARDIAN_KEY", addr, chainId, blockNumber, pubKey))` (`:38-56`).

**Liveness:** `/upcheck` → immediate 200; `sign-exit` → immediate, pure-local;
`keygen` & `validate-custody` → "eventually", blocking on the CVM-agent socket /
SessionRegistry RPC (no app-level timeout, so a stuck dependency blocks
**indefinitely** — G5).

**Documentation-vs-reality gaps:**

- **G1 (central).** README claims validate-custody "verifies the validator
  enclave's CVM session evidence on-chain." The entire `verify_session_evidence`
  path is gated on the **client-controlled** `request.verify_session`
  (`mod.rs:71`), and **reef passes `verify_session: false`**
  (`new_registration.rs:266`) with empty session/workload fields. **In
  production today, attestation/session/workload verification never runs** —
  production safety reduces to S1–S6 (cryptographic well-formedness of the
  payload), which says nothing about *where the payload came from*.
- **G3.** Three sources disagree on where verification happens: README (guardian,
  on-chain, when flagged), `docs/migration-guide-tdx.md:244` (guardian does
  "structural-only", defers to caller — now **stale/false**: the code really does
  the `eth_call`), and the code. The documented contract is internally
  inconsistent.
- **G4.** The deposit-forbidden check and slashing protection in
  `shared/mod.rs` are real but **unreachable through the guardian router** (no
  `/api/v1/eth2/sign` route registered on the guardian).

## 7. Failure & degradation modes (highest-value Antithesis surface)

1. **F1 (Critical) — attestation bypass via `verify_session` flag.** See G1.
   Client controls whether the on-chain check runs at all.
2. **F7/F8/F11 (High) — input-reachable panics, no panic middleware → connection
   reset with no response.**
   - F8: `BlsKeygenPayload::signature()` does `copy_from_slice` into `[u8;96]`
     with **no length guard** (`types.rs:167-173`) — panics on a bad-length
     `signature`. **Reachable on every validate-custody call regardless of
     `verify_session`.**
   - F7: `verify_session_evidence` does `dd_root.copy_from_slice(hex::decode(
     deposit_data_root))` with **no length check** (`mod.rs:135-136`) — panics on
     `deposit_data_root` ≠ 32 bytes.
   - F11: `ListKeysResponse::new` indexes `pk[0..2]` (`types.rs:50`) — panics on a
     short/odd filename present in the keys dir (GET list handlers).
3. **F3 (High) — CVM agent has no timeout/retry.** A hung socket hangs keygen
   forever. `CVM_AGENT_STUB=true` returns empty evidence with no error and **no
   production guard** (W11) — easy to ship mis-stubbed and look healthy (201).
4. **F9 (High) — orphaned secrets from write-before-success** (see §3).
5. **F10 (High) — session liveness never checked.** `verify_session_evidence`
   reads only `session.workloadId`; it **never checks `isActive`, `expiresAt`,
   or `isSessionActive`** (the struct/fns exist, `session_registry.rs:30-45`).
   Expired/revoked sessions would be accepted — *and* the workload check itself
   is skipped whenever `workload_id` is empty (which reef sends).
6. **F2 (Med) — `.zip()` silent truncation** in
   `build_validator_remote_attestation_payload` (`shared/mod.rs:200`): if
   `guardian_eth_pub_keys.len() != bls_enc_priv_key_shares.len()`, trailing
   shares are silently dropped from the hashed payload (fail-closed but an
   unvalidated assumption that can desync the reconstructed preimage).
7. **F4 (Low) — SessionRegistry RPC fails closed** (errors → 500); availability
   impact only, never fail-open.
8. **F5 (Med) — non-atomic key writes** (see §3).

## 8. Unproven assumptions

- The CVM agent socket is always present and responsive (no timeout).
- The SessionRegistry RPC always agrees with the contract and is always reachable.
- Input vectors are equal length (`.zip`) and hex fields are well-formed and
  correctly sized (panics otherwise).
- The enclave ETH key already exists on disk when validate-custody is called
  (instance affinity; no cross-VM recovery).
- Sessions are fresh (no expiry/active check).
- The caller is trusted: `validator_index`, `guardian_module_address`,
  `chain_id`, `fork_version`, and `verify_session` are all taken from the request
  with no independent check (the custody path trusts the payload-supplied
  `fork_version`, `types.rs:175-192`, while the BLS signing path uses the trusted
  `AppState` genesis version — an inconsistency, W13).
- The Rust-side preimage construction byte-for-byte matches both the Automata CVM
  signer and the Solidity verifier (three hand-rolled builders; one uses
  `threshold().to_be_bytes()` on a **platform-dependent `usize`**, `shared/
  mod.rs:212`).

## 9. Cross-cutting / wildcard findings (binding & contract-drift)

These are the deepest attestation-focus concerns; they live at layer boundaries
and need cross-repo confirmation.

- **W0 — contract preimage drift (liveness-breaking; confirmed byte-for-byte).**
  Property discovery (focus 4) verified this against the on-disk
  `puffer-contracts@feat/tdx` checkout: the guardian signs a **7-field** approve
  preimage `(moduleAddr, chainId, validatorIndex, blsPubKey, wc, depositSig,
  ddRoot)` (`mod.rs:322-336`), but `LibGuardianMessages._getBeaconDepositMessage
  ToBeSigned` verifies only **5 fields** (no addr/chainId) → every guardian's
  correct signature is rejected on-chain → **provisioning stalls protocol-wide on
  that branch**. The 7-field match exists only on
  `origin/fix/increase-guardian-signatures-security`, which in turn lacks the
  `ROTATE_GUARDIAN_KEY` rotate format the current guardian emits — so **no single
  contract branch satisfies both of this guardian's preimages**. The remaining
  unknown is *which bytecode is actually deployed* (Open Question 1). Commit
  `61bf1d2` ("use correct payload") shows this drifted once before.
- **W1 — approval bound to neither the attestation nor the share/guardian.**
  `approve_custody` commits only to group-level deposit fields — **not**
  `session_id`, **not** share index `i`, **not** the enclave's own pubkey. Two
  requests differing only in `verify_session` produce **byte-identical**
  signatures.
- **W2/W3 — no nonce/replay protection; `validator_index` attacker-chosen.**
  Confirmed there is no nonce/deadline/used-signature mapping on-chain (only the
  monotonic `nextToBeProvisioned`). `verify_custody` never references
  `validator_index` (`mod.rs:281-310`) — a caller can obtain an approval binding
  an arbitrary `validator_index` to a keyshare.
- **W4 — guardian identity inferred by trial decryption, never pinned.**
  `verify_custody` accepts "whichever share decrypts" and never checks the
  enclave's own pubkey is the one at `guardian_eth_pub_keys[i]`.
- **W10 — `sign-exit` has zero authorization.** It mints exit-signature shares
  for any caller-supplied `(blsPubKeySet, guardian_index, validator_index)` with
  a stored share, epoch hardcoded to 0, no custody/ownership/attestation gate.
  Combined with the inert attestation + write-before-success persistence, an
  unattested-then-persisted share becomes a live exit-signing capability.

## 10. Attack surfaces (Antithesis "where bugs hide" lens)

- **Timing/interleaving:** concurrent validate-custody + sign-exit on the same
  BLS share (torn read); concurrent load → head-of-line blocking stalling
  `/upcheck`.
- **Partial failure:** CVM agent up but RPC down; keyshare persisted then crash
  before approval; one of N shares decrypts then a later step errors.
- **Stale/asynchronous state:** expired/revoked CVM session accepted (F10);
  contract preimage drift (W0).
- **Malformed input under fault:** length-mismatched hex fields → panic → silent
  connection reset (F7/F8/F11).
- **Crash recovery:** truncated/lost key files after VM reset; orphaned secrets.

## 11. Bug history & regression targets

The **entire attestation/signature path was rewritten SGX → TDX in the last ~4
months** on branch `feat/tdx-improve-signature` — it is the freshest, riskiest,
least-settled code:

- `io/session_registry.rs` (new, commit `5941761`): alloy `sol!` bindings +
  off-chain `eth_call`. Written once, never refactored, **never tested for real
  behavior**.
- `verify_session_evidence` (+141 lines, `5941761`); `approve_custody` +
  `attest_new_eth_key_with_blockhash` reworked in `d119699` to add
  `guardian_module_address` + `chain_id` to the signed ABI payload. Commit
  `61bf1d2` ("use correct payload…") shows this preimage was **already wrong
  once**.
- `io/cvm_agent.rs` / `remote_attestation.rs` (`c82feee`): new Unix-socket IPC;
  compile-time `sgx` flag replaced by runtime `CVM_AGENT_STUB`.
- Four trailing "Version update" commits = deploy thrashing; nothing settled.

## 12. Existing test strategy & coverage gaps

- **The priority risk area has zero real coverage.** Tests never exercise the
  real CVM agent or real SessionRegistry: `verify_session=false` /
  `do_remote_attestation=false` are hardcoded (`src/client/tests/mod.rs:19,52`);
  `test_verify_session_evidence_structural_checks` only asserts three fields are
  non-empty and **explicitly stops before any on-chain call** — the "integration
  test against a forked chain" it references **does not exist**.
- `test_approve_custody` only asserts `.is_ok()` — it **never checks the signed
  bytes match the on-chain `GuardianModule` format** (the exact thing that
  churned in `d119699`/`61bf1d2`). Highest-value, lowest-coverage correctness
  property.
- The only E2E custody integration test (`src/enclave/test/integration.rs`) is
  **entirely commented out** (`// TODO: fix this test`).
- **CI is non-functional:** `.github/workflows/unit-test.yml` triggers only on
  changes to the workflow file itself (`paths:` filter) and installs
  `--default-toolchain none`. No working CI gate has run on the attestation code.
- Untested branches: registry returns `false`; RPC error fail-open/closed;
  workload-id mismatch; malformed `session_id`/`deposit_data_root` (panic); the
  decrypt loop accepting a wrong share; exit replay.

**Antithesis adds the most value** exactly here: the on-chain verification path,
the byte-exact preimage formats, the panic/timeout/partial-failure behavior, and
the concurrency/crash scenarios — none of which the existing tests touch.

## 13. Product context & impact

- **Threshold model:** each validator's BLS key is split M-of-N across guardians;
  the threshold is enforced on-chain (`GuardianModule.get_threshold()`, checked in
  reef `utils/contract.rs`). Tests use small sets (e.g. 7-of-8 / index 1);
  **the production M and N are unknown** (open question). All guardians run the
  **same binary** → a single guardian's liveness failure is tolerated below
  threshold, but a *systematic* verification flaw is forgeable up to quorum =
  **catastrophic**.
- **Harm per failure mode:** (a) bad custody approval → validator gets
  activated/funded but is **un-ejectable / funds stuck**; (b) refusing a valid
  custody → registration stalls (liveness, tolerated below N−M); (c) lost/corrupt
  share → moves the set toward losing the ability to ever exit/eject; (d)
  unauthorized/forced `sign-exit` reaching M guardians → **force-ejects live
  validators**.
- **Attestation is currently OFF in production**, so the root of trust today is
  *not* TDX attestation — it is the unauthenticated reef/BFF orchestrator plus
  the cryptographic well-formedness checks S1–S6.

## Assumptions (analysis-level)

- "Guardian binary" = the routes in `bin/guardian.rs` only. Shared-module code
  reachable only via the validator/secure-signer routers is out of scope.
- The deployed image runs from CWD `/` (so `./data` ⇒ `/data` volume); if that
  assumption breaks, key persistence silently relocates.
- The Automata CVM agent signs `keccak256(payload)` exactly as the comments
  claim; not verified byte-for-byte in this pass.

## Open Questions (catalog-wide; see also per-property evidence files)

1. **Contract alignment (W0):** which `puffer-contracts` branch/commit is the
   guardian's co-signature actually verified against in the target deployment? If
   none matches the current 7-field preimage, custody approval is broken
   end-to-end (liveness) regardless of anything Antithesis does. **Needs human
   input.**
2. **Production M-of-N** threshold and guardian count — determines blast radius
   of a systematic bypass. **Needs human input.**
3. **Is `verify_session` intended to be flipped on?** If yes, F10
   (no isActive/expiresAt check) and G2 (empty session fields) are live bugs; if
   no, the entire SessionRegistry path is dead code and the attestation story is
   aspirational. **Needs human input / product decision.**
4. **Is `/guardian/v1/validate-custody` reachable by untrusted clients** in the
   TDX network topology, or only by reef over a private link? This decides
   whether F1/F7/F8/W1–W3 are remotely exploitable or defense-in-depth. **Needs
   human input.**
5. **Can `CVM_AGENT_STUB=true` reach a production image?** If the build doesn't
   strip it, attestation can be silently disabled. **Needs human input / build
   audit.**
6. Is there any reconciliation/GC for orphaned key files (F9)? No code path found.

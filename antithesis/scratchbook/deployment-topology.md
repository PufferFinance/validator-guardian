---
sut_path: /home/fawad/puffer/projects/validator-guardian
commit: f7dbd88d99de21a6d8caba5f7de8216624ab1098
updated: 2026-06-30
external_references:
  - path: /home/fawad/puffer/projects/puffer-contracts
    why: source for SessionRegistry + GuardianModule, deployable into a local anvil so the workload can observe real on-chain accept/reject (tests custody-preimage-matches-onchain-verifier / W0).
  - path: /home/fawad/puffer/projects/atakit
    why: reference for the CVM-agent session-signing protocol the mock CVM agent must emulate; the socket contract for /app/cvm-agent.sock.
  - path: /home/fawad/puffer/projects/reef
    why: reference call patterns the workload should replicate (and deliberately vary, e.g. verify_session=true).
---

# Deployment Topology — `guardian` binary

Minimal container topology that covers the guardian's code paths under Antithesis.
Goal: the fewest containers/links that still let us inject the faults the property
catalog needs. Three containers (one of them optional-but-recommended for the
priority attestation properties).

```text
 +------------------------+        HTTP :9001         +---------------------------------+
 | workload (client)      | ------------------------> | guardian (service / SUT)        |
 | - antithesis test cmds | <------------------------ |  - guardian binary              |
 | - puffersecuresigner   |     approvals / errors    |  - mock CVM agent (helper,      |
 |   as a library         |                           |    unix socket /app/...sock)    |
 +-----------+------------+                           +---------------+-----------------+
             |                                                        |
             | (also acts as the validator + reef:                   | HTTP JSON-RPC :8545
             |  registers session, submits provision_node)           v
             |                                          +---------------------------------+
             +----------------------------------------> | session-registry (dependency)   |
                       eth_call / tx                    |  anvil + SessionRegistry +      |
                                                        |  GuardianModule  (or mock RPC)  |
                                                        +---------------------------------+
```

## Component 1 — `guardian` (service / SUT)

| | |
|---|---|
| **Role** | Service (the SUT) |
| **Image** | **Reuse `container/Dockerfile`** with `--build-arg BINARY_NAME=guardian` (multi-stage cargo-chef build already present; `container/guardian/docker-compose.yml` is a working starting point). Add the Antithesis Rust SDK + build instrumentation (below). |
| **Runs** | `guardian` binary on port `9001` (set `GUARDIAN_PORT=9001`), plus an in-container **mock CVM agent** helper listening on the unix socket. |
| **Env** | `GUARDIAN_PORT=9001`, `GENESIS_FORK_VERSION=00000000`, `CVM_AGENT_SOCKET_PATH=/app/cvm-agent.sock`, `SESSION_REGISTRY_RPC_URL=http://session-registry:8545`, `SESSION_REGISTRY_ADDRESS=<deployed addr>`. **Do not set `CVM_AGENT_STUB`** for normal runs (we want real-ish attestation via the mock); set it only in the dedicated test that exercises `cvm-stub-never-in-production`. |
| **Persistence** | `guardian-data` volume at `/data` (keys live in `/data/keys/{eth,bls}_keys`). See "Persistence & termination" below — the volume's durability across container kills is a deliberate knob. |
| **Connections** | inbound HTTP from workload (`:9001`); outbound JSON-RPC to `session-registry` (`:8545`); inbound unix-socket from its own mock CVM agent (in-container). |
| **Replicas** | **1.** The guardian is single-process; cross-guardian behavior is aggregated on-chain (catalog P4), so a single instance plus the on-chain contract covers the threshold/agreement properties. (A 3-guardian variant is discussed under "Replica decisions" and is **not** recommended for the first harness.) |

### The mock CVM agent (in-container helper)

The real CVM agent is a unix-socket sidecar at `/app/cvm-agent.sock`
(`io/cvm_agent.rs`). **A unix socket is in-container, so Antithesis network faults
cannot reach it** — therefore we run a **controllable mock** as a helper process
in the guardian container, not as a separate faultable container. The mock:

- Implements the `automata-cvm-agent` `sign_message` socket protocol (see the
  atakit reference; the SUT calls `CvmAgent::new(path).sign_message(data)`),
  returning `{session_id, signature, session_key, owner_key}`.
- Signs `keccak256(data)` with a **fixed session keypair** known to the harness,
  so the workload can register that key / its signatures in `session-registry`.
- Has a **controllable mode** driven by a control file (or signal) that a
  `singleton_driver_`/custom-fault command flips: `ok` (sign normally), `error`
  (return failure), `hang` (sleep ≫ any timeout — exercises
  [[dependency-hang-makes-progress]]), `empty` (the stub-equivalent — pairs with
  the `CVM_AGENT_STUB` test for [[cvm-stub-never-in-production]]).
- A file or directory under the test template prefixed `helper_` if shipped via
  the test template; otherwise baked into the guardian image.

> If building the mock is too costly initially, the cheaper fallback is
> `CVM_AGENT_STUB=true` — but that **only** exercises the empty-evidence path and
> makes the keygen attestation inert, so it cannot test the happy attestation
> path or the hang/error behaviors. Prefer the controllable mock.

## Component 2 — `session-registry` (dependency)

| | |
|---|---|
| **Role** | Dependency (the on-chain verifier the guardian calls via `eth_call`, and that the workload submits `provision_node` to) |
| **Image (recommended)** | **`ghcr.io/foundry-rs/foundry` (anvil)** with `SessionRegistry` + `GuardianModule` deployed from `puffer-contracts` at container start. This is the only way to actually test [[custody-preimage-matches-onchain-verifier]] (W0) and [[approval-binding-and-replay-resistance]] end-to-end: the workload submits the guardian's approval to `GuardianModule` and observes accept/reject. |
| **Image (minimal)** | A small **mock JSON-RPC** server (any language) implementing `verifySessionSignature(...)→bool` and `getSession(...)→CVMSession` with controllable returns. Cheaper, but cannot catch on-chain preimage drift — it can only test the guardian's fail-closed behavior and the structural checks. |
| **Runs** | Ethereum JSON-RPC on `:8545`. |
| **Connections** | inbound from guardian (`eth_call`) and from workload (session registration + `provision_node` tx). |
| **Replicas** | 1. |
| **Why a separate container** | It speaks **TCP**, so isolating it lets Antithesis inject partitions/latency/bad-node between guardian↔RPC — exactly what [[dependency-hang-makes-progress]] and [[session-verification-fails-closed]] need. |

## Component 3 — `workload` (client / test driver)

| | |
|---|---|
| **Role** | Client — runs the Antithesis test commands that exercise the guardian. |
| **Image** | New Rust container that **depends on `puffersecuresigner` as a library** (reuse `GuardianClient`/`ClientBuilder`, `BlsKeygenPayload`, the validator-side BLS threshold keygen + ECIES share-encryption, deposit-message construction). Add the Antithesis Rust SDK for assertions. |
| **Runs** | `setup_complete` once ready, then stays alive; Antithesis runs the test-template commands at `/opt/antithesis/test/v1/{name}/`. |
| **What it does** | Plays both the **validator/coral side** (generate a BLS key set, encrypt shares to the guardian's enclave pubkey, build deposit data, build + sign the validator attestation payload with a session key registered in `session-registry`) and the **reef side** (call `POST /eth/v1/keygen`, `validate-custody`, `sign-exit`; submit the returned approval to `GuardianModule`). It deliberately varies `verify_session` ∈ {false,true}, `validator_index`, `guardian_index`, fork_version, share counts, and injects malformed-length fields. |
| **Connections** | HTTP to guardian (`:9001`); JSON-RPC/tx to session-registry (`:8545`). |
| **Replicas** | 1 (can drive many logical validators/guardians sequentially and concurrently). |

## Mapping properties → topology needs (sanity check)

- **Attestation (Category A/B):** require `verify_session=true` paths ⇒ need
  `session-registry` reachable and a session key the workload controls (mock CVM
  agent + registered session). The on-chain preimage property needs the **anvil +
  real contracts** variant.
- **Liveness/availability (Category G):** `dependency-hang-makes-progress` needs
  the CVM mock `hang` mode and network faults on the guardian↔RPC link;
  `upcheck-live-under-load` needs concurrent driver load + node-throttle/CPU-mod.
- **Persistence/recovery (Category F):** `key-write-durable-or-rejected`,
  `no-orphaned-unacknowledged-secret` (crash), `missing-state-request-fails-clean`
  (restart) **require node-termination** (see below).
- **Input robustness (Category H):** workload-only (malformed inputs, encoding
  variants) — no extra topology.

## Faults to enable / be aware of

- **Node termination/restart is OFF by default** — **request it enabled** for the
  guardian container, or Categories F crash/restart properties pass vacuously.
- **guardian ↔ session-registry** is the one network link that matters — partition/
  latency/bad-node here drive the RPC fail-closed and liveness properties.
- **CVM agent unix socket cannot be network-faulted** — its failure modes come
  from the controllable mock + custom faults, not Antithesis network faults.
- **Clock jitter** is the negative control for [[guardian-routes-time-independent]].
- **Thread-pause / CPU-modulation** create the interleavings for
  [[torn-read-never-yields-wrong-key]] and the starvation for
  [[upcheck-live-under-load]].

## Persistence & termination knob

`/data` is a named volume. Two test modes, both valuable:
- **Durable** (volume survives kill): tests crash-recovery of on-disk keys
  ([[key-write-durable-or-rejected]] — does a key survive a kill mid-`fs::write`?).
- **Ephemeral** (fresh FS on restart, which Antithesis node-termination may
  produce): tests [[missing-state-request-fails-clean]] — validate-custody/sign-exit
  for a pre-restart key must fail cleanly, and `/upcheck` must not falsely report
  healthy on a key-less guardian.

## Known image issue to fix for clean termination testing

`container/Dockerfile:38` uses **shell-form `ENTRYPOINT`** (`/usr/local/bin/${BINARY_NAME}`)
→ the binary runs as a child of `/bin/sh`, **not PID 1**, so it does not receive
`SIGTERM`/`SIGINT` directly. For meaningful graceful-shutdown / termination tests,
switch to exec-form (`ENTRYPOINT ["/usr/local/bin/guardian"]`) or add an init.
Also note the binary currently **discards the bind `Result`** and has **no
graceful-shutdown hook** (see [[startup-bind-failure-not-silent]]).

## SDK selection

- **Workload container:** Antithesis **Rust SDK** (`antithesis-sdk` crate) —
  required to emit `assert_*`, `Sometimes`, `Reachable`, and lifecycle
  (`setup_complete`) signals.
- **Guardian (SUT) container:** add the **Rust SDK** too. Many catalog properties
  need **SUT-side instrumentation** that cannot be observed from the workload
  (e.g. "share matched at index i" for [[stored-share-matches-committed-pubkey-share]],
  the panic catch for [[malformed-input-never-panics]], the "verification ran"
  flag for [[attestation-verified-before-approval]]). The SDK is a no-op outside
  Antithesis, so it is safe to leave in the guardian build. **It is not currently
  a dependency** (`existing-assertions.md`) — adding it is the first
  `antithesis-setup`/`antithesis-workload` task. For deepest coverage, build the
  guardian with Antithesis compile-time instrumentation (coverage) per the
  Instrumentation docs.

## Evaluation refinements (applied)

The property-evaluation pass surfaced topology-affecting refinements:

- **R2 — the W0 / preimage properties don't need a full chain to catch drift.** The
  *primary*, cheapest check for [[custody-preimage-matches-onchain-verifier]] and
  [[keygen-evidence-accepted-onchain]] is a **no-chain Rust/`alloy sol!` reference
  encoder** differentially compared to `approve_custody` / the rotate preimage —
  this catches the W0 drift deterministically with no anvil. A **minimal
  GuardianModule-only** anvil deploy (skipping the full SessionRegistry tree, since
  `validateProvisionNode` recovers against `enclaveAddress` independent of session
  verification) is the secondary form. The full SessionRegistry deploy (~5 registry
  deps + AccessManager, ~500-700 LoC Solidity wiring) is a separately-sized work
  item, only needed for the verify-session accept path.
- **R7 — the verify-session accept path's session key is the most fragile
  precondition.** The in-repo sim CVM agent (`SimCvmAgent`) generates its session
  signing key with `SigningKey::random(OsRng)` **per registration** — it is NOT a
  fixed harness keypair. The harness must either patch atakit to seed the key, or
  extract the random pubkey at runtime and dynamically register it in
  `session-registry` before each verify-path custody call (and keep the session_id/
  key binding consistent, since the mock is a dependency of BOTH the SUT keygen and
  the workload's validator-side payload producer). The fail-closed / error / hang
  variants do NOT need this (a mock RPC returning false/erroring suffices).
- **R8 — the mock CVM agent cannot catch *real-agent* byte drift.** Because the mock
  signs `keccak256` by construction, it always agrees with the guardian's
  reconstruction — so [[rotate-key-preimage-matches-onchain]] /
  [[attestation-payload-reconstruction-matches]] get their differential value from
  the Rust↔contract reference encoder, not from the mock. The mock validates the
  *guardian↔mock* loop, not the *guardian↔production-DCAP-agent* boundary.
- **R6 — termination realism:** switch `container/Dockerfile` to **exec-form
  ENTRYPOINT** (the binary is not PID 1 today, so it won't receive SIGTERM), and
  author **two `/data` configs** (durable vs reset) for the crash-recovery vs
  fresh-restart properties. Confirm node-termination is enabled on the tenant.
- **R9 — `malformed-input-never-panics` high-fidelity form** needs `tower-http`
  (catch-panic) added; absent that, use a lower-fidelity workload-side
  connection-reset proxy (which can't distinguish a panic from a network fault).

## Open Questions

- **anvil vs mock RPC** for `session-registry`: the anvil+contracts variant is
  required to actually test W0 on-chain, but adds build complexity (compile +
  deploy `puffer-contracts`). Decide based on whether the on-chain preimage
  property is in the first harness scope. *(Recommended: yes — it is the
  highest-severity property.)*
- Does the mock CVM agent need to emulate the **real DCAP** evidence shape, or is
  the session-signing subset enough? (See the atakit investigation; the guardian
  only calls `sign_message`, so the session-signing subset should suffice.)
- Production guardian count / threshold (catalog Q2) — only matters if we later
  build the multi-guardian variant.

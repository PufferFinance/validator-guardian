# Property: keygen / validate-custody make progress (succeed or return an error) and never hang forever when CVM agent / SessionRegistry RPC is unavailable

slug: `dependency-unavailable-makes-progress`
focus: (3) Failure Recovery — dependency-failure recovery / liveness (F3, G5)
priority: HIGH
confidence: HIGH (timeout absence confirmed in SUT + dependency crate)

## Origin / evidence

- **CVM agent has NO timeout/retry** on the request path used by keygen:
  - `src/io/cvm_agent.rs:32-45` `sign_with_session`: builds `CvmAgent::new(socket_path)` **per request** (no pool, line 36) and `client.sign_message(message).await?` — no `tokio::time::timeout` wrapper.
  - The dependency itself does not impose one: `automata-cvm-agent` `client/cvm_agent.rs:117` `UnixStream::connect(&self.socket_path).await` and the subsequent send/read have **no `set_read_timeout`/`set_write_timeout`/`timeout(...)`** (grep confirmed: only `connect`, no timeout calls). So a socket that accepts the connection but never replies hangs `sign_message().await` **indefinitely**.
  - Reached from keygen: `src/enclave/guardian/mod.rs:56` `AttestationEvidence::new(&payload).await?` → `remote_attestation.rs:36` `sign_with_session(data).await?`.
- **SessionRegistry RPC has no app-level timeout** (only relevant if `verify_session=true`):
  - `src/io/session_registry.rs:64-65, 89-90` rebuild the alloy provider **per call** (`ProviderBuilder::new().connect_http(...)`), no `.timeout(...)`. Reached from `verify_session_evidence` (`mod.rs:184` and `:201`) — up to 2 RPC round-trips per verifying request. On error: `.context(...)?` → 500 (fail-closed). On a black-hole (TCP accept, no response) the HTTP client's own default may or may not bound it — alloy/reqwest default has no request timeout unless set.
- **No request-level timeout at the server either** — no `TimeoutLayer` (grep confirmed zero `TimeoutLayer`/`tower_http`), so axum will not abort a stuck handler. A hung CVM socket pins the worker indefinitely (couples to head-of-line-blocking.md).
- Stub escape hatch: `CVM_AGENT_STUB=true` short-circuits to empty evidence (returns 201) — but the guardian `env` file sets only `RUST_LOG` + `GUARDIAN_PORT` (confirmed `container/guardian/env`), so in the default/production config the stub is OFF and the real socket path is used.

## What breaks

- A CVM agent that is **slow / hung / partitioned** (Antithesis: node-hang/throttle on the cvm-agent sidecar, or bind-mounted socket that accepts but stalls) makes keygen **never return** — no 200, no 500, the connection just stays open. reef's guardian client (`src/client/guardian.rs:38-43`) `await`s the response with whatever timeout reef's `reqwest::Client` was built with; the guardian itself provides no bound, so if reef has no timeout either, the whole provisioning step stalls.
- Same for validate-custody when `verify_session=true` and the RPC endpoint black-holes.
- This is a **liveness** failure: the desired property is that within a bounded recovery window after the dependency is restored (or after it fails fast), the operation reaches a terminal HTTP status.

## Antithesis angle

- Inject CVM-agent unavailability (kill/hang the sidecar, or make the socket connect-but-stall) and/or SessionRegistry RPC partition (network fault: bad-node / partition / latency).
- Liveness property using the mid-run recovery window: with `ANTITHESIS_STOP_FAULTS`, after the dependency is restored, a pending/new keygen must **eventually** return a terminal status (`eventually_`/`finally_` style). Equivalent assertion: `Sometimes(keygen returned a terminal HTTP status while/after CVM was faulted)` proves progress is *possible*; the stronger `eventually_always` proves it’s *guaranteed* once faults stop.
- Negative companion: `Reachable("keygen still pending > N seconds with CVM agent unreachable")` documents the no-timeout hang — this is the bug evidence.

## Fault dependency

- Does **NOT** require node-termination. Driven by **network faults (partition/latency/bad-node)** and **node-hang/throttle** on the CVM-agent / RPC dependency. Liveness verified via `ANTITHESIS_STOP_FAULTS` recovery window.
- (Optional) combine with node-termination of the *guardian* to also test that a killed guardian mid-CVM-call doesn't leave reef hung — but the core property doesn't need it.

## Timing / config deps

- "Hang forever" specifically needs a connect-but-no-reply condition (a refused connection fails fast → clean 500, which is fine). Antithesis socket/network fault that accepts then stalls is the trigger.
- For the RPC variant, `verify_session=true` + `SESSION_REGISTRY_RPC_URL`/`SESSION_REGISTRY_ADDRESS` set is required; reef sends `verify_session=false` today (sut-analysis G1), so in the *current* production config only the CVM-agent (keygen) hang is live. Flag this: the RPC-hang variant is conditional on a config that production doesn't use yet.

## Instrumentation status: MISSING (mostly workload-observable)

- Progress/terminal-status is observable from the workload (did the HTTP call return a status within the recovery window?) — so the liveness assertion can largely live in the workload without SUT changes. Mark **partial**.
- A precise "handler entered CVM call" / "handler returned" event pair as SUT instrumentation would let Antithesis attribute the stall to the CVM step specifically (vs. transport) — nice-to-have, MISSING.
- antithesis-sdk not yet a dependency.

## Open questions (why they matter)

- ~~Does reef set a client-side timeout on guardian calls?~~ **RESOLVED: NO.**
  reef's guardian calls go through the SUT's own `puffersecuresigner` client
  (`ClientBuilder`), and that builder constructs `reqwest::Client::new()` with **no
  `.timeout()` / `.connect_timeout()`** — reqwest's default is *no* request
  timeout. There is no `tokio::time::timeout` wrapper on the reef side either. So a
  guardian CVM-hang stalls reef's provisioning indefinitely with no recovery short
  of operator action. See Investigation Log.
- Should the guardian wrap CVM/RPC calls in `tokio::time::timeout` so a stuck dependency fails closed (500) instead of hanging? **Matters: a fail-fast 500 is recoverable (reef retries); an indefinite hang is not.** This is the concrete fix the property argues for. *(still open — design recommendation, needs human input)*
- Is `CVM_AGENT_STUB=true` reachable in a production image (would mask the hang by returning fake 201)? sut-analysis Open Q5. **Matters: a mis-stubbed build looks healthy and never exercises the real timeout-less path.** *(still open — not in scope of assigned reef questions)*

---

## Investigation Log

### Does reef set a client-side timeout on guardian HTTP calls (keygen, validate-custody, sign-exit)?
- **Examined:**
  - `reef/reef-guardian/src/utils/secure_signer.rs` (full) — only a health check;
    no client construction.
  - All reef sign/keygen/custody call sites that build the client:
    `eject_validator.rs:187-190`, `rotate_guardian_key.rs:65-69`,
    `provision_or_skip/handler.rs:62`, `health_check.rs:19` — all use
    `ClientBuilder::new().guardian_url(...).build()`.
  - The `ClientBuilder` itself: reef pulls `puffersecuresigner` as a git dep
    (`reef-guardian/Cargo.toml:39`, `reef-lib/Cargo.toml:36`, branch
    `feat/tdx-improve-signature` = the SUT), so the "reef client" IS the SUT
    client. Source: `src/client/mod.rs:36-83`, `src/client/guardian.rs` (full).
  - `grep` for `timeout`, `Client::builder`, `reqwest::Client` across reef and the
    SUT client module.
- **Found:**
  - `src/client/mod.rs:51-52` `ClientBuilder::build()` →
    `let client = Arc::new(reqwest::Client::new());`. **No `.timeout(...)`, no
    `.connect_timeout(...)`** anywhere in the builder. The single `reqwest::Client`
    is cloned into the guardian / validator / secure_signer clients.
  - `src/client/guardian.rs` — every method (`health` `:14`, `attest_fresh_eth_key`
    `:26` = keygen, `validate_custody` `:72`, `sign_exit` `:93`) is a bare
    `self.client.post(...).json(...).send().await?` with **no `tokio::time::timeout`
    wrapper**. (reef's separate `reqwest::Client::new()` calls in
    `reef-lib/src/services/beacon/*` are for beacon, also untimed, but those are
    not the guardian path.)
  - reqwest's `Client::new()` has no default request timeout, so these awaits block
    until the OS TCP layer gives up — effectively unbounded against a
    connect-but-no-reply (black-hole) peer.
- **Not found:** Any `.timeout`, `TimeoutLayer`, or `tokio::time::timeout` on
  reef's guardian-client path.
- **Conclusion:** **reef sets NO client-side timeout on guardian calls.** Combined
  with the SUT's own lack of a CVM-agent / RPC timeout, a hung CVM agent during
  keygen propagates an *indefinite* stall up through reef's provisioning flow —
  the whole provisioning step hangs, not just one guardian's sub-call. This
  confirms the `dependency-hang-makes-progress` liveness concern end-to-end and
  strengthens (does not change) the property: the hang is unrecoverable without
  operator action on either side.

# Property: the on-disk key directory does not grow without bound across repeated keygen / failed custody (resource exhaustion)

slug: `unbounded-keydir-growth`
focus: (5) Resource Boundaries — unbounded key-dir growth / orphan accumulation
priority: MEDIUM
confidence: MEDIUM-HIGH

## Origin / evidence

- **Every keygen writes a NEW file.** `POST /eth/v1/keygen` → `eth_key_gen()` generates a fresh random secp256k1 key (`src/crypto/eth_keys.rs:11-20`) and writes it under its own (compressed-pubkey) filename (`save_eth_key` → `write_eth_key`, `:87-95`, `key_management.rs:17-23`). Distinct keys ⇒ distinct filenames ⇒ **monotonic file growth**; nothing is ever overwritten or deleted on this path.
- **Every successful validate-custody writes a BLS share file** (`mod.rs:82-85`). Re-running custody for the same share overwrites (same pk filename) — bounded; but different validators ⇒ new files, monotonic.
- **Orphans accumulate** (see orphaned-secret-write-before-success.md): failed/killed keygen and failed-after-write custody leave files that are never returned to or cleaned up.
- **No GC / no cap:** `delete_eth_key`/`delete_bls_key` exist (`key_management.rs:84-95`) but are **never called from any guardian route** (confirmed). No size cap, no LRU, no retention. Disk is a fixed encrypted 10GB volume (`atakit.json` `guardian-data` 10GB).
- **No request body-size limit** (no `DefaultBodyLimit` / `RequestBodyLimitLayer`; grep = NONE) — a large validate-custody body (e.g. huge `bls_enc_priv_key_shares` vector) is fully buffered into memory before handling, and the trial-decrypt loop (`mod.rs:291`) iterates the whole vector. Memory + CPU amplification per request with no cap.
- **No connection reuse to dependencies:** `CvmAgent::new` per request (`cvm_agent.rs:36`), alloy provider rebuilt per call (`session_registry.rs:64,89`). Under repeated failed CVM/RPC attempts this means a fresh UnixStream / HTTP connection each time → FD/socket churn (and with no timeout, leaked/long-lived stuck connections — couples to dependency-unavailable-makes-progress.md).

## What breaks

- Key files (each tiny, ~64-66 hex chars) grow slowly, but `list_eth_keys`/`list_bls_keys` (`fs::read_dir` over the whole dir, `key_management.rs:117-136`) get **O(files) slower** every keygen; `GET /eth/v1/keygen` (list handler) degrades and is itself blocking on a worker (head-of-line-blocking.md). At extreme counts, inode/dir-entry pressure on a 10GB encrypted volume.
- The bigger resource risk is **FD/socket exhaustion** under sustained CVM/RPC failure: each retry opens a new connection with no timeout to bound how long a stuck one lingers. Repeated hung connections can exhaust file descriptors / ephemeral ports for the process.
- **Unbounded request body** → memory spike / CPU blowup from a single crafted request (no body limit + trial-decrypt loop length = attacker/caller-controlled).

## Antithesis angle

- Workload: hammer `POST /eth/v1/keygen` and (failing) validate-custody in a loop while injecting CVM/RPC faults; periodically measure key-dir file count, process FD count, RSS, and `GET /eth/v1/keygen` latency.
- Properties (mostly SUT-instrumented gauges / workload-observable):
  - `Reachable("eth key file count exceeded N after repeated keygen with no intervening delete")` — documents monotonic growth + absence of GC.
  - `Sometimes("process FD count stayed bounded across K failed CVM attempts")` — healthy; failure to ever hold ⇒ FD leak.
  - `Always("validate-custody with an oversized body is rejected or bounded")` — currently expected to FAIL (no body limit) → this is a finding, not a guaranteed invariant; frame as `Reachable("oversized custody body fully buffered/processed")` to prove the missing limit.
- Liveness: after `ANTITHESIS_STOP_FAULTS`, FD count / memory return to baseline (no permanent leak).

## Fault dependency

- Does **NOT** require node-termination. Uses network/dependency faults (to drive repeated failed CVM/RPC attempts → FD/socket churn) + workload volume. The pure key-dir-growth variant needs no faults at all (just repeated keygen) — fault-free reachability.

## Timing / config deps

- Disk cap 10GB; key files are tiny, so disk-full from keys alone is a long-horizon concern — the *latency degradation* of list + *FD exhaustion* under failure are the near-term effects. Worth bounding the run length / request rate to make growth observable.
- FD limit = container `ulimit -n` (unknown; deployment config). **Matters for when socket churn actually exhausts.**

## Instrumentation status: MISSING / partial

- File counts, FD counts, RSS are observable from the workload/sidecar without SUT changes (read `/proc/<pid>/fd`, `ls` the key dir) — **partial**, workload-side.
- A clean Antithesis assertion ("no FD leak", "key dir bounded") benefits from SUT-emitted gauges but isn't required. antithesis-sdk not yet a dependency.

## Open questions (why they matter)

- **(partial: FD lifecycle resolved in code; numeric ulimit needs human input.)**
  What is the process FD ulimit and does a stuck (no-timeout) CVM/RPC connection
  actually leak the FD or get dropped when the future is cancelled? **Code part
  resolved (see Investigation Log):** the CVM-agent client has *no* timeout and
  axum 0.6 does *not* cancel the handler future on client disconnect, so a stuck
  connection holds its FD for the full (unbounded) duration of the hang — it is not
  promptly reclaimed by client abort. The numeric `ulimit -n` is **not set in the
  repo** (no `LimitNOFILE`/`ulimit` in `atakit.json` or the compose file) → Docker
  default (~1024) **(needs human input / deployment fact).**
- Is there any retention/rotation expectation for old enclave ETH keys (each keygen
  rotates the guardian key per the ROTATE_GUARDIAN_KEY tag)? If keys are meant to
  rotate frequently, the dir grows fast and GC absence is a real operational bug.
  **Matters for growth rate. (needs human input — intended rotation frequency is an
  operational policy, not in code; code confirms no GC/cap exists.)**
- ~~Should there be a `DefaultBodyLimit` on validate-custody?~~ **RESOLVED (factual
  part): there is currently NO body-size limit on any route** (no `DefaultBodyLimit`/
  `RequestBodyLimitLayer`, no `tower`/`tower-http` dep, no layers on the router at
  all — `bin/guardian.rs:35-65`). axum 0.6's default `Json`/body extractor has **no
  byte cap** without an explicit limit, so a large `validate-custody` body is fully
  buffered. *Whether* one should be added is a design call, but the absence is now
  confirmed in code. See Investigation Log.

## Investigation Log

### (a) Is there a body-size limit on any route?

**Examined:** `bin/guardian.rs:35-65` (router), `Cargo.toml`. Grepped repo for
`DefaultBodyLimit`, `RequestBodyLimit`, `tower`, `tower_http`, `content-length`,
`max_body`, `body_limit`.

**Found / Not found:** **No body-size limit anywhere.** The router is four
`.route()`s + `.with_state()` with **zero `.layer()` calls**; there is no
`tower`/`tower-http` dependency. All three guardian POST handlers take
`Json(request): Json<…>` (`validate_custody.rs:6`, `sign_exit.rs:6`,
`attest_fresh_eth_key_with_blockhash.rs:6`), which in axum 0.6 buffers the entire
request body with **no default byte cap** when no `DefaultBodyLimit` is configured.

**Conclusion:** Confirmed — **no body-size limit on any of the four routes.** A
single large `validate-custody` body (huge `bls_enc_priv_key_shares` vector) is
fully read into memory and then iterated O(N) by the ECIES loop (see
[[upcheck-live-under-load]] Investigation Log) — memory + CPU amplification with no
cap, exactly as the property describes. Property's `Reachable("oversized custody
body fully buffered/processed")` framing is correct and confirmed.

### (b) alloy provider + CvmAgent per-request; FD/connection lifecycle on a hung/aborted request

**Examined:** `io/cvm_agent.rs` (`sign_with_session`/`CvmAgent::new`), the locked
atakit `CvmAgent` client (git rev `9f9bfea`,
`crates/automata-cvm-agent/src/client/cvm_agent.rs` `post()`), `io/session_registry.rs`
(`verify_session_signature`/`get_session`), `io/remote_attestation.rs`.

**Found:**
- **Per-request construction, no pooling — confirmed.**
  - CVM agent: `CvmAgent::new(socket_path)` is created fresh on every keygen /
    session-sign call (`cvm_agent.rs:36`), and inside `post()` a brand-new
    `UnixStream::connect` + hyper `http1::handshake` is opened per call. No client
    or connection is cached.
  - alloy: `ProviderBuilder::new().connect_http(rpc_url)` is rebuilt inside *each*
    of `verify_session_signature` (`session_registry.rs:64`) and `get_session`
    (`session_registry.rs:89`) — a fresh HTTP provider/connection per RPC call.
- **No timeout anywhere in the CVM client `post()`** — `UnixStream::connect`,
  `http1::handshake`, `send_request`, and `into_body().collect()` are all unbounded
  `.await`s (grep for `timeout`/`Duration` in that file = none). The alloy `.call()`
  paths likewise set no app-level timeout. So a hung dependency pins the worker (and
  holds the socket FD) **indefinitely** — corroborates the G5 no-timeout finding.
- **FD lifecycle on a hung/aborted request:** the FD (UnixStream / HTTP socket) is
  owned by the handler future's locals and is closed by RAII only when that future
  completes/errors or is dropped. In axum 0.6 the handler future is **not cancelled
  when the client disconnects** (axum 0.6 has no client-disconnect-driven cancel;
  the task runs to completion). Therefore a client abort does **not** promptly
  reclaim the FD — it lingers for the whole duration of the (unbounded) dependency
  hang. The FD is only reclaimed if/when the underlying transport itself eventually
  errors. Note also `post()` `tokio::spawn`s the hyper connection driver, so the
  connection's lifetime is tied to that spawned task plus the request future.

**Not found:** any connection pool, any `tokio::time::timeout` wrapper, any
client-disconnect cancellation, any explicit FD/socket cleanup on error paths beyond
RAII.

**Conclusion:** Per-request providers/agents with **no pooling and no timeout**,
and FDs that are **not** reclaimed on client abort (axum 0.6 runs the handler to
completion). So sustained CVM/RPC failure → each retry opens a fresh socket that
lingers for the full hang, i.e. long-lived FDs accumulate bounded only by the
process `ulimit -n`. The "FD leak on stuck connections" risk is **real and
code-confirmed**; the only missing piece is the numeric ulimit (deployment fact).

### (c) Key dir GC / cap

**Examined:** `io/key_management.rs` (full file), all call sites of `delete_eth_key`/
`delete_bls_key` (grep over `src/` excluding tests).

**Found / Not found:** `delete_eth_key`/`delete_bls_key` exist
(`key_management.rs:84-95`) but are **never called from any guardian route** (only
referenced in `#[cfg(test)]`). No size cap, LRU, retention, or background GC exists.
Every keygen `write_eth_key`s a new uniquely-named file; nothing prunes. Disk is the
fixed 10GB encrypted `guardian-data` volume (`atakit.json:40-47`,
`container/guardian/docker-compose.yml:15`).

**Conclusion:** Confirmed — **no GC and no cap on the key dir.** Monotonic growth
per keygen; `list_eth_keys` (`fs::read_dir` over the whole dir) gets O(files)
slower over time. Property unchanged, confirmed.

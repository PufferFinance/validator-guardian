# Property: After a restart that loses prior on-disk state, dependent ops fail cleanly (500), never panic / never silently succeed with no key

slug: `instance-affinity-restart-fail-closed`
focus: (3) Failure Recovery — instance affinity / restart with empty/new volume
priority: MEDIUM-HIGH
confidence: HIGH

## Origin / evidence

- Instance affinity is structural (sut-analysis §2):
  - validate-custody needs a **prior keygen on the same disk** — it reads the enclave ETH sk first: `src/enclave/guardian/mod.rs:64` `fetch_eth_key(&eth_pk_to_hex(&request.guardian_enclave_public_key))`; on miss the `let Ok(...) else` at `:64-68` returns `Err(anyhow!("Could not fetch guardian enclave public key"))` → handled as 500 (`validate_custody.rs:12-19`).
  - sign-exit needs a **prior validate-custody** — `mod.rs:361` `fetch_bls_sk(&pk_hex)?`; on miss `read_bls_key` → `fs::read` errors → `?` → 500 (`sign_exit.rs:12-19`).
- State is local to one VM/volume; **no replication, no cross-VM recovery** (sut-analysis §2/§3).
- Volume config: `container/guardian/docker-compose.yml` mounts named volume `guardian-data:/data`. Under normal docker restart this persists; under an Antithesis **node-termination** the SUT may be handed a **fresh, non-durable filesystem** (the prompt explicitly flags this).
- Keys live at `./data/keys/{eth,bls}_keys/` (`src/constants.rs:1-4`), resolved relative to CWD `/` ⇒ `/data/...` only because the runtime image sets no WORKDIR (sut-analysis §3). If CWD ever differs, keys silently relocate and every dependent read misses.

## What breaks

After a restart onto an empty/new volume (or a wrong CWD):
- **Expected/safe:** validate-custody returns a clean 500 ("Could not fetch guardian enclave public key"); sign-exit returns a clean 500 ("Unable to read secret key"). reef sees `Err` and treats it as a guardian that hasn't keygen'd yet → re-runs keygen. This is the **fail-closed, recoverable** outcome we want to confirm.
- **Unsafe outcomes to rule out:**
  - A panic instead of a clean error (would drop the TCP connection with no HTTP response — there is **no CatchPanicLayer**, sut-analysis §2). Known input-reachable panics F7/F8/F11 (`types.rs:167-173` signature length, `mod.rs:135-136` dd_root length, `types.rs:50` `pk[0..2]` on the GET list) can fire on these same requests *regardless* of the missing key, and a panic-on-empty-state looks like a hang/reset to reef rather than a recoverable "no key yet".
  - `GET /eth/v1/keygen` on a fresh empty dir: `list_fnames` (`key_management.rs:117-118`) does `fs::read_dir` which **errors if the dir doesn't exist** → `bail!("No keys saved in dir")` → 500, instead of an empty list. So a freshly-restarted guardian answers "list keys" with a 500, not `[]` — a reconciliation client can't distinguish "empty" from "broken".
- **/upcheck stays 200 regardless** (`shared::handlers::health`, unconditional) — so the guardian *looks* healthy after losing all its keys; only a dependent call reveals the loss. This is the dangerous bit: liveness probe is decoupled from actual key-state readiness.

## Antithesis angle

- Workload: keygen → validate-custody → sign-exit, then **node-termination with a reset/empty volume**, then re-issue validate-custody and sign-exit for the *old* pubkeys.
- Properties:
  - `Always("dependent op after state loss returns an HTTP response")` — i.e. never a panic/connection-reset; it must be a 500 (or 200), not a dropped connection. SUT-instrumented (panic guard) — see below.
  - `AlwaysOrUnreachable("validate-custody on missing eth key returns 500, not 200")` — fail-closed: never mint an approval without the enclave key.
  - `Reachable("GET /eth/v1/keygen returned 500 on a fresh/empty key dir")` — documents the empty-vs-error ambiguity (candidate bug; ideally should be `[]`).
  - Liveness: after `ANTITHESIS_STOP_FAULTS`, a fresh keygen+custody+exit cycle succeeds (recovery works).

## Fault dependency

**REQUIRES node-termination (kill/restart) — DISABLED by default — for the realistic "lost volume after crash" scenario.** The empty-dir/`fs::read_dir` error variant can also be reached fault-free by simply starting a guardian and calling validate-custody/sign-exit/list before any keygen (no fault needed) — so there is a fault-free reachability proof for the fail-closed behavior, and the termination fault is needed only for the *recovery-after-crash* angle.

## Timing / config deps

- Whether termination yields a fresh FS vs. the persisted named volume is the pivotal config (same open question as in key-write-durable-or-rejected.md).
- The panic variants (F7/F8/F11) depend on malformed input fields, independent of state — so the "always returns an HTTP response" assertion should be tested with both well-formed and malformed payloads under restart.

## Instrumentation status: MISSING / partial

- "Returns an HTTP response, never a connection reset from a panic" is an availability/anchor property best served by adding a `CatchPanicLayer` (SUT change) AND an Antithesis `assert_unreachable` in the panic landing site, or by the workload asserting at the client that every request gets a status code (no transport error) — partial: the client-side check is doable in the workload without SUT changes, but distinguishing a panic-reset from a real network fault needs the panic guard.
- The fail-closed 500-not-200 assertion is observable from the workload (HTTP status) — can be a workload-side `assert_always` without SUT instrumentation. Mark: **partial (workload-observable)**.

## Open questions (why they matter)

- Does Antithesis node-termination give the guardian a fresh empty `/data` or the persisted volume? **Decides which variant is live.**
- Should `GET /eth/v1/keygen` return `[]` for an empty/absent dir instead of 500? **Matters for any reconciliation/health tooling that lists keys to detect state loss.**
- Is there any startup self-check that the expected keys exist (readiness gate distinct from `/upcheck`)? None found. **Matters: `/upcheck` 200 on a key-less guardian is a false-healthy signal that can keep a useless guardian in the M-of-N rotation.**


---

> **[merged]** consolidated from discovery focus file `prop-focus-6/lifecycle-out-of-order-request-fails-clean.md`

# Property: out-of-order request fails cleanly (no panic, no hang)

Slug: `lifecycle-out-of-order-request-fails-clean`
Focus: (8) Lifecycle Transitions — instance affinity / ordering
Instrumentation: **MISSING** (no Antithesis SDK in repo; needs SUT-side `assert_*`)

## Origin / the ordering assumption
The guardian's three stateful endpoints form a strict on-disk causal chain on a
single VM/volume (no replication, no in-memory cache — every op re-reads disk):

1. `POST /eth/v1/keygen` *produces* the enclave ETH sk on disk.
2. `POST /guardian/v1/validate-custody` *consumes* that ETH sk (to ECIES-decrypt
   the share) and *produces* a BLS share on disk.
3. `POST /guardian/v1/sign-exit` *consumes* the BLS share.

The implicit assumption (sut-analysis §2, §8) is that each request arrives only
after its prerequisite state was created on the same disk. Antithesis will issue
them out of order (e.g. validate-custody before any keygen; sign-exit before any
validate-custody; any request against a fresh/empty volume after a node restart).

## Files / functions / lines
- Ordering gate 1 (validate-custody needs prior keygen):
  `src/enclave/guardian/mod.rs:64-68` — `fetch_eth_key(...)` → `else { return Err(anyhow!("Could not fetch guardian enclave public key")) }`.
  - `fetch_eth_key`: `src/crypto/eth_keys.rs:98-102` → `read_eth_key` →
    `read_key` (`src/io/key_management.rs:49-52`) → `fs::read(...).with_context("Unable to read secret key")?`.
  - Missing file ⇒ `Err` (NOT a panic).
- Ordering gate 2 (sign-exit needs prior validate-custody):
  `src/enclave/guardian/mod.rs:352-364` — `sign_voluntary_exit_message` →
  `fetch_bls_sk(&pk_hex)?` (`src/crypto/bls_keys.rs:58-65`) → `read_bls_key` →
  `read_key` → `fs::read(...)?`. Missing file ⇒ `Err`.
- Error → HTTP mapping (clean 500, no panic):
  - validate-custody handler `src/enclave/guardian/handlers/validate_custody.rs:12-19` (`Err(e) => 500`).
  - sign-exit handler `src/enclave/guardian/handlers/sign_exit.rs:12-20` (`Err(e) => 500`).

## What breaks / the risk
Two distinct failure shapes can occur for an out-of-order request:
- **Clean (expected):** missing-prerequisite file ⇒ `anyhow::Err` ⇒ HTTP 500
  with a body. This is the desired behavior — the property asserts it ALWAYS
  takes this path.
- **Panic (violation):** the SAME out-of-order request, if it also carries a
  malformed field, reaches an un-guarded `copy_from_slice` BEFORE the disk read
  gate is even evaluated, panicking the handler. With **no `CatchPanicLayer`**
  (sut-analysis §2; confirmed `grep CatchPanic` = 0), a panic unwinds out of the
  axum handler and **drops the TCP connection with no HTTP response** — caller
  sees a connection reset, not a clean 500. Panic vectors that interleave with
  ordering:
  - **F8** `BlsKeygenPayload::signature()` `src/enclave/types.rs:167-173`:
    `sig_bytes.copy_from_slice(&hex::decode(&sanitized)?)` into `[u8;96]` with no
    length guard. Reached on EVERY validate-custody via `verify_deposit_message`
    → `keygen_payload.signature()` (`mod.rs:265`). Note: in
    `verify_and_sign_custody_received` the `fetch_eth_key` gate (`:64`) runs
    FIRST, so on an empty disk the request fails clean *before* F8; but once a
    keygen exists, a malformed `signature` length panics.
  - **F7** `verify_session_evidence` `src/enclave/guardian/mod.rs:135-136`:
    `dd_root.copy_from_slice(&hex::decode(deposit_data_root)?)` no length check —
    only on the `verify_session=true` branch (reef sends false).

So the property is really: **for any request whose causal prerequisite is
absent, the guardian returns a clean HTTP error (4xx/5xx) and never panics or
hangs.** The interesting bug is the panic interleaving, not the happy 500.

## Antithesis angle
- Workload: drive endpoints in deliberately wrong order against a fresh volume,
  and after node restart (see fault dependency). Assert the response is a
  well-formed HTTP status (not a dropped connection / reset).
- Type: `Always(received an HTTP response for every out-of-order request)` —
  best expressed SUT-side by wrapping handler bodies so a panic is caught and
  converted, then asserting `AlwaysOrUnreachable` that the panic branch is the
  Unreachable one. Practically: add `CatchPanicLayer` + an Antithesis
  `assert_unreachable!("guardian handler panicked")` inside the catch.
- `Reachable("validate-custody rejected: enclave ETH key absent")` and
  `Reachable("sign-exit rejected: BLS share absent")` confirm the clean-reject
  path is actually exercised (liveness of the error path).

## Fault dependency
- **Node termination (restart) — DISABLED by default; THIS PROPERTY NEEDS IT
  ENABLED.** A restart with a fresh/empty `guardian-data` volume, or after the
  volume is mounted but before any keygen, is the most realistic way to reach
  "prerequisite state absent" in production (a re-provisioned VM). Without
  restart, the out-of-order condition can still be produced by workload ordering
  alone, but restart is what makes it a genuine lifecycle property.
- Clock/network faults not required.

## Open questions
- Is `/guardian/v1/validate-custody` reachable by untrusted clients in the TDX
  topology, or only reef over a private link? (sut-analysis OQ4). If untrusted,
  the F8 panic-reset is a remote DoS, raising priority. **Needs human input.**
- Does any orchestration retry on a 500 from an out-of-order request, and could a
  retry storm against a not-yet-keygen'd guardian amplify head-of-line blocking?
  (No retry policy found in guardian; reef-side TBD — see distributed evidence.)

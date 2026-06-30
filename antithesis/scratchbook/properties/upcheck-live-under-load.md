# Property: /upcheck stays live under concurrent blocking load (head-of-line blocking of tokio workers)

slug: `upcheck-liveness-under-blocking-load`
focus: (2) Concurrency — blocking work on async workers starves all tasks

## Origin
Assigned lead "Head-of-line blocking." Confirmed: heavy work is synchronous &
blocking and runs directly on tokio worker threads; no `spawn_blocking` /
`block_in_place` anywhere (grep = 0). Enough concurrent in-flight blocking
handlers can occupy every worker thread and starve even the trivial `/upcheck`,
making the only liveness/health signal go unresponsive while the process is
"alive."

## Code path (files + functions + lines)
- Runtime: `src/bin/guardian.rs:4` `#[tokio::main]` (no args) = multi-thread
  runtime; `worker_threads` defaults to number of CPU cores. `tokio = { "1",
  features=["full"] }`. No `TimeoutLayer`, no `ConcurrencyLimitLayer`, no tower
  layers at all (`bin/guardian.rs:35-65`).
- Blocking work executed **on the async worker** inside handlers:
  - `validate-custody` → `verify_and_sign_custody_received` (`mod.rs:60-104`):
    blocking `fs::read` of eth key (`mod.rs:64` → `read_key` = `fs::read`),
    synchronous ECIES trial-decrypt loop `verify_custody` (`mod.rs:281-310`,
    `decrypt_sk_share` does `ecies::decrypt` per share, types.rs:211-226),
    BLS pk-set/sig verification, blocking `fs::write` (`mod.rs:82`). When
    `verify_session=true`, also two awaited RPC round-trips with **no timeout**
    (`io/session_registry.rs`, sut-analysis §5) and a CVM-agent socket call with
    no timeout on keygen.
  - `sign-exit` → blocking `fs::read` + BLS sign (`mod.rs:352-394`).
  - `keygen` → CVM-agent unix-socket call, **no timeout** (`mod.rs:56`,
    `io/remote_attestation.rs`).
- `/upcheck` → `shared::handlers::health::handler` (`health.rs:3-5`) returns 200
  with zero work — but it is still a task that must be polled by some worker.

## What breaks
The blocking calls (`fs::*`, `ecies::decrypt` loop, BLS ops, and especially the
no-timeout socket/RPC awaits) hold a worker thread for the whole duration of the
synchronous section. If concurrent in-flight requests ≥ worker_threads, every
worker is parked in synchronous code and the runtime has no free thread to poll
the `/upcheck` future. Health probes time out → orchestrator may consider the
guardian down (or, worse, *not* down because TCP connect still succeeds), even
though the process is healthy. With no `ConcurrencyLimitLayer`/`TimeoutLayer`,
there's no backpressure or shedding — load simply converts to latency on all
endpoints including health.

The no-timeout dependency calls (CVM agent socket, SessionRegistry RPC) make this
sharply worse: a single hung dependency pins a worker **indefinitely** (G5 in
sut-analysis), so a small number of hung custody/keygen requests can permanently
remove workers from rotation, and `/upcheck` degrades/stalls with far fewer than
"num_cores" concurrent requests.

## Exact interleaving / timing window
- Spin up N concurrent `validate-custody` (or `keygen`) where N ≥ worker_threads.
- Inject **io-latency / node-throttle** (slow encrypted disk) or a **CVM-agent /
  RPC hang** (network partition / bad-node to the socket/RPC) so each handler's
  synchronous/await section is long-lived.
- While saturated, issue `/upcheck`. Window: for the duration all workers are in
  blocking sections, the health task cannot be polled.

## Antithesis angle
- Faults: **node-throttle / CPU-modulation** (slows the synchronous CPU work);
  **io latency** on the data volume; **network partition / bad-node** against the
  CVM-agent socket and SessionRegistry RPC to trigger the no-timeout hang.
- Liveness property (use `eventually_`/`finally_` style): `Sometimes`/`Reachable`
  that `/upcheck` returns 200 *while* ≥ worker_threads custody/keygen requests are
  in flight; and an eventual-liveness assertion that after `ANTITHESIS_STOP_FAULTS`
  (faults lifted), `/upcheck` returns 200 within a bound (recovers — no permanent
  worker leak).
- The contrast that makes it meaningful: assert `/upcheck` 200 is `Reachable`
  under concurrent load (it should be, on a correct design) and watch whether it
  becomes *unreachable* during saturation — i.e. liveness regression detection.

## Instrumentation status
**MISSING / partial.** `/upcheck` already exists as a health signal (present in
SUT), but no Antithesis liveness assertion wraps it. SUT-side: the cleanest fix is
`tokio::task::spawn_blocking` around `fs`/ECIES/BLS and per-dependency timeouts;
the assertion can be purely workload-side (probe `/upcheck` under load) without
SUT changes, which makes this property cheap to test even before any fix.

## Open questions (why they matter)
- Production `worker_threads` count (depends on the CVM vCPU count) sets how many
  concurrent blocking requests are needed to starve `/upcheck`. Unknown.
  *Matters for sizing the workload's concurrency.*
- Are CVM-agent/RPC calls truly unbounded (no OS-level socket timeout)? Code shows
  no app timeout; if the underlying transport has a default timeout the
  "indefinite" pin becomes "long but bounded." *Matters for whether this is a
  permanent or transient liveness loss.*
- Is `/upcheck` the probe the deployment actually uses for liveness, and does it
  distinguish "process up" from "can serve work"? If the orchestrator only checks
  TCP connect, head-of-line blocking is invisible to it. *Matters for real-world
  impact.*


---

> **[merged]** consolidated from discovery focus file `prop-focus-3/head-of-line-blocking-upcheck.md`

# Property: /upcheck (and other endpoints) stay responsive under concurrent blocking load — no runtime starvation

slug: `head-of-line-blocking-upcheck`
focus: (5) Resource Boundaries — head-of-line blocking / runtime starvation
priority: MEDIUM-HIGH
confidence: HIGH (no spawn_blocking confirmed)

## Origin / evidence

- The guardian is `#[tokio::main]` multi-threaded (`src/bin/guardian.rs:4`), axum 0.6.20, but **all heavy work in handlers is synchronous/blocking and runs directly on the async worker threads**:
  - Blocking FS: `fs::read`/`fs::write`/`fs::read_dir` in `src/io/key_management.rs` (`:13, :50, :118`) — on a GCP-encrypted disk these can be slow.
  - CPU-bound crypto on the worker: the ECIES trial-decrypt loop `verify_custody` (`src/enclave/guardian/mod.rs:291-307`) iterates over every encrypted share calling `decrypt_sk_share`; BLS verify/sign (`verify_deposit_message` `:264`, `sign_vem` `:391`); keccak/ABI encode.
  - The CVM-agent `await` in keygen (`mod.rs:56`) can block a worker indefinitely (see dependency-unavailable-makes-progress.md).
- **No `spawn_blocking` / `block_in_place` anywhere** — repo-wide grep returned NONE. So blocking work is never moved off the async workers.
- **No `TimeoutLayer`, no concurrency limit, no `tower` layers at all** (`src/bin/guardian.rs:35-65`; grep for `TimeoutLayer`/`tower_http` = NONE). Nothing sheds or bounds load.
- `/upcheck` is the **only true liveness signal** (`shared::handlers::health`, unconditional 200) and reef gates every guardian call on it (`reef-guardian .../new_registration.rs:151` `check_enclave_health`, and `eject_validator.rs:191`). If `/upcheck` can't be serviced because all workers are blocked, reef concludes the guardian is **down** and skips it from the M-of-N set.

## What breaks

- Under a burst of concurrent validate-custody/keygen requests on a slow/throttled disk (Antithesis: node-throttle / CPU-mod / disk-latency), every tokio worker can be parked in a blocking syscall or a long crypto loop. With the default worker count = #cores (c3-standard-4 ⇒ 4), it takes only ~4 concurrent stuck handlers to **starve the runtime**, so even the trivial `/upcheck` GET queues behind them and times out at the caller.
- Consequence: a guardian that is *actually alive and making progress* is misclassified as dead by reef's health check → dropped from quorum participation → if this hits ≥ N−M guardians simultaneously (they all run the same binary, same disk profile), **provisioning/eject liveness fails for the whole set** (sut-analysis §13).
- A single hung CVM call (no timeout) permanently consumes one worker, lowering the threshold for starvation.

## Antithesis angle

- Workload: drive concurrent validate-custody + keygen load while periodically probing `/upcheck`, under node-throttle / CPU-mod / disk-latency faults.
- Properties:
  - `Sometimes("/upcheck responded within Xms while >=K custody/keygen requests were in-flight under throttle")` — liveness: proves the runtime can still service the health probe under load. A *failure* of this Sometimes to ever hold (paired with an `eventually_` health-recovers after `ANTITHESIS_STOP_FAULTS`) is the starvation signal.
  - `Reachable("/upcheck latency exceeded reef's health-check timeout while handlers blocked")` — documents the head-of-line stall.
  - Liveness: after faults stop, `/upcheck` returns to fast 200 (recovery).

## Fault dependency

- Does **NOT** require node-termination. Uses **node-throttle / CPU-modification / thread-pause** and **disk/network latency** faults, plus concurrency in the workload. `ANTITHESIS_STOP_FAULTS` for the recovery half.

## Timing / config deps

- Worker count = tokio default (#cpus). On the configured `c3-standard-4` that's ~4 — low, so starvation is easy to provoke. If the Antithesis env gives more/fewer cores the K threshold shifts.
- Encrypted-disk slowness is the realistic amplifier; Antithesis disk-latency fault stands in for it.
- Sensitive to whether reef's health-check has a tight timeout (cross-repo) — determines at what latency `/upcheck` "fails".

## Instrumentation status: partial (mostly workload-observable)

- `/upcheck` latency under load is fully workload-observable (probe + measure) — no SUT change needed for the core assertion. Mark **partial**.
- An optional SUT gauge of "in-flight blocking handlers" / "worker pool saturation" would make the starvation attribution crisp, but is not required. MISSING (nice-to-have).
- Real fix this argues for (out of scope for the property, but the "why"): wrap blocking FS/crypto in `spawn_blocking` and add a `TimeoutLayer` + concurrency limit.

## Open questions (why they matter)

- What is reef's `/upcheck` health-check timeout and does a single slow probe drop the guardian from quorum, or is it retried/debounced? **Matters: determines whether transient starvation = transient skip vs. sustained exclusion.**
- How many tokio worker threads does the deployed runtime actually have? **Matters: directly sets the K (concurrent blocked requests) needed to starve `/upcheck`.**
- ~~Does any single endpoint dominate latency (the ECIES trial-decrypt loop scales with number of shares = N guardians)?~~ **RESOLVED (yes, and N is caller-controlled & uncapped).** See Investigation Log. The `validate-custody` ECIES trial-decrypt loop is O(N) full ECIES decryptions on the blocking worker, N = `bls_enc_priv_key_shares.len()` straight from the request body with **no cap** — so a caller can make a single `validate-custody` handler arbitrarily expensive, dominating latency and starving `/upcheck` with very few concurrent requests.

## Investigation Log

### ECIES trial-decrypt loop scaling — does cost scale with N, and is N caller-controlled / unbounded?

**Examined:** `verify_custody` (`enclave/guardian/mod.rs:281-310`), `decrypt_sk_share`
(`enclave/types.rs:211-226`), `envelope_decrypt` → `ecies::decrypt`
(`crypto/eth_keys.rs:153-159`), the `ValidateCustodyRequest`/`BlsKeygenPayload`
structs (`enclave/types.rs:85-147`), and every reference to
`bls_enc_priv_key_shares` in `src/` (grep).

**Found:**
- The loop is `for i in 0..keygen_payload.bls_enc_priv_key_shares.len()`
  (`mod.rs:291`). Each iteration calls `decrypt_sk_share(i, …)` which hex-decodes
  share `i` and runs a **full ECIES decryption** (`ecies::decrypt`:
  ephemeral-key ECDH + HKDF + AES-GCM) at `eth_keys.rs:155`. So per-request cost is
  **O(N) ECIES decryptions**, N = number of encrypted shares.
- **Worst case is the full N:** the loop only early-returns when a decrypted share's
  public-key-share matches `pk_set.public_key_share(i)` (`mod.rs:302-305`). A caller
  whose shares never match (or all fail to decrypt) forces the loop to run all N
  iterations and then `bail!` (`mod.rs:309`). On each *successful* decrypt it also
  rebuilds `public_key_set()` from the bls_pub_key_set hex inside the loop
  (`mod.rs:300`) — extra per-iteration parse cost.
- **N is caller-controlled and uncapped.** `bls_enc_priv_key_shares: Vec<String>`
  is a plain request-body field (`types.rs:140`). There is **no length check / cap**
  on it anywhere before or inside `verify_custody` (the only validations are the
  per-element `.get(i)` and a hex decode). With no `DefaultBodyLimit` on the router
  (see bounded-resource-usage), the whole vector is buffered and iterated.
- This runs **synchronously on the tokio worker** (no `spawn_blocking`), and runs
  *after* `verify_deposit_message` but is the dominant CPU cost in the handler.

**Not found:** any cap on `bls_enc_priv_key_shares.len()`, any `spawn_blocking`
around the loop, any short-circuit other than the pk-share match.

**Conclusion:** The ECIES trial-decrypt loop **does** scale linearly with
caller-supplied N and is the latency-dominant section of `validate-custody`. Because
N is unbounded request input, a single crafted `validate-custody` can occupy a
worker for an attacker-chosen duration — so `/upcheck` head-of-line starvation can be
provoked with far fewer than `worker_threads` *honest* requests (a few large-N
requests suffice). This resolves the open question and strengthens both the
starvation argument here and the CPU/memory-amplification finding in
[[bounded-resource-usage]].

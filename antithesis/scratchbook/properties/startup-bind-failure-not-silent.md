# Property: startup binds successfully or fails loudly; no silent half-up server

Slug: `lifecycle-startup-readiness-no-silent-degradation`
Focus: (8) Lifecycle Transitions — startup / readiness gating
Instrumentation: **MISSING** (needs SUT-side instrumentation at startup)

## Origin / the transition assumption
Two coupled startup defects:

1. **Bind result discarded → silent exit.** `src/bin/guardian.rs:69-71`:
   ```rust
   _ = axum::Server::bind(&addr)
       .serve(app.into_make_service())
       .await;
   ```
   The `Result` of `bind(...).serve(...)` is bound to `_`. If the port is already
   in use, the address is unavailable, or `serve` returns an error, `main`
   returns and the process **exits 0 with no log line and no panic** (the
   `println!` at `:26-29` already fired, so logs *claim* it started). Antithesis
   sees a healthy-looking start followed by a vanished server.

2. **No readiness gating.** `/upcheck` (`src/enclave/shared/handlers/health.rs:3-5`)
   returns `200 OK` unconditionally — it inspects NO state. It returns 200:
   - before any ETH key exists (so validate-custody will 500),
   - before `SESSION_REGISTRY_*` env is set (so any `verify_session=true` request
     will 500),
   - even if the CVM agent socket `/app/cvm-agent.sock` is absent/unresponsive
     (keygen will hang/500).
   The only thing 200 proves is "the tokio runtime is alive and a worker is not
   blocked" — it is a liveness signal, NOT a readiness signal.

Startup also hard-`.expect`s on env: `GUARDIAN_PORT` parse (`:9-14` `"BAD PORT"`)
and `GENESIS_FORK_VERSION` hex decode (`:16-24` `"Bad genesis_fork_version"`).
These are loud panic-exits (acceptable). `CVM_AGENT_STUB`, `SESSION_REGISTRY_*`
are read lazily per-request, NOT at startup — so a misconfig is invisible until
the first real request.

## Files / functions / lines
- `src/bin/guardian.rs:9-24` env `.expect`s; `:26-29` premature "Starting" log;
  `:67` `SocketAddr::from(([0,0,0,0], port))`; `:69-71` discarded bind/serve.
- `src/enclave/shared/handlers/health.rs:3-5` unconditional 200.
- `src/io/remote_attestation.rs:30-43` `CVM_AGENT_STUB` checked per keygen, not at
  startup.
- `src/enclave/guardian/mod.rs:169-177` `SESSION_REGISTRY_*` read per verifying
  request, not at startup.
- Container: `container/Dockerfile:38` `ENTRYPOINT /usr/local/bin/${BINARY_NAME}`
  (shell form → binary is a child of `/bin/sh -c`, not PID 1). `EXPOSE 9001`;
  `container/guardian/env` sets only `RUST_LOG`, `GUARDIAN_PORT=9001` — **no
  `SESSION_REGISTRY_*`, no `CVM_AGENT_STUB`** (sut-analysis §5).

## What breaks / the risk
- **Silent-exit on bind failure:** orchestration/health checks that only probe
  `/upcheck` after a delay can mistake "process gone" for "still starting", or a
  supervisor restarts into a crash-loop with no diagnostic. In a guardian set
  this silently removes one guardian from the quorum (moves toward losing
  threshold) with no alert.
- **Readiness gap:** `/upcheck`=200 while the guardian cannot actually serve
  keygen/custody means reef (or a load balancer) routes real traffic to a node
  that 500s every request — a correlated, silent partial outage across the set if
  several guardians come up under-provisioned.

## Antithesis angle
- `Reachable("guardian server bound and is serving")` — SUT-side: instrument the
  point AFTER a successful bind (requires capturing the bind `Result` instead of
  `_`). If the assertion is never reached in a run where the workload expected a
  server, that's the silent-exit signal.
- `Always(if /upcheck==200 then a subsequent keygen/custody does not fail with a
  "prerequisite missing / config absent" 500)` — a readiness invariant. Today
  this is **violated by construction** at cold start, so it is a true bug-finder;
  expressed as `assert_always` comparing upcheck status to first-request outcome
  in the workload, OR SUT-side by making `/upcheck` actually gate on
  key-dir/agent reachability and asserting consistency.
- `Sometimes(bind failed and was surfaced)` — only meaningful once the bind
  `Result` is no longer discarded.

## Fault dependency
- **Node termination (restart) — DISABLED by default; helpful but not strictly
  required.** Restart + a port still held (TIME_WAIT / a lingering sibling) is the
  natural way to trigger the discarded-bind silent exit.
- **Network throttle / node hang on the CVM-agent socket** exercises the
  readiness gap (upcheck 200 while keygen hangs) without restart.
- Clock faults not required.

## Open questions
- Does the deployment rely on `/upcheck` for load-balancer/orchestrator readiness?
  If yes, the missing readiness gate is operationally severe. **Needs human
  input.** (Compose declares no `healthcheck:`; `atakit.json` declares none.)
- Is the discarded-bind an intentional fire-and-forget or an oversight? Capturing
  the `Result` and logging is a near-zero-cost fix worth flagging to the team.
- Shell-form `ENTRYPOINT` means the guardian is not PID 1 and won't get
  forwarded signals — interacts with the shutdown property below.

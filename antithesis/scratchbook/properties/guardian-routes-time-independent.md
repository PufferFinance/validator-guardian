# W7-5 — The guardian has ZERO time dependence: a confirm/deny clock-jitter property (and the one place it bites)

## Origin / intuition
The wildcard prompt asked: *would clock jitter (an Antithesis fault) interact
with any time-based logic — session expiry, RPC timeouts, anything?* I traced
every time source reachable from the three guardian routes. **Finding: the
guardian binary is essentially time-blind.** No wall-clock read, no monotonic
timer, no TTL, no timeout, no expiry check, and the exit epoch is a hardcoded
constant. This is itself a testable property — *clock-jitter immunity* — and it
is a double-edged one: immunity is a robustness win for liveness, but it is the
direct mechanism behind two safety gaps (no session-freshness check; an exit
signed for a fixed epoch regardless of when it is requested).

## Files / functions / line numbers (the audit)
- **Exit epoch is a constant** — `src/enclave/guardian/mod.rs:364`
  `sign_vem(sk, 0, ...)` — epoch hardcoded `0`. The exit signing root therefore
  does not depend on current time/epoch at all (`mod.rs:372-393`).
- **No session expiry / liveness read** — `verify_session_evidence`
  (`mod.rs:115-225`) reads only `session.workloadId` (`:213`); it never reads
  `expiresAt` / `isActive` / `isSessionActive`. (SUT F10 — restated here only to
  locate the single time-relevant field that is *ignored*.)
- **No app-level RPC/socket timeout** — `io/session_registry.rs` (alloy default,
  no `.timeout()`), `io/cvm_agent.rs:32-37` (`CvmAgent::post` has no timeout per
  SUT §5). So there is no client-side deadline whose firing could be perturbed
  by clock skew — a stuck dependency hangs *indefinitely* regardless of the
  clock.
- **No nonce/deadline in approvals** — `approve_custody` (`mod.rs:312-350`) and
  the keygen preimage (`mod.rs:45-51`) commit to `block_number` (a chain height,
  not wall time) but no timestamp/deadline.
- **No `tokio::time`, `Instant`, `SystemTime`, `Duration` in guardian routes** —
  grep across the guardian path: the only "time-ish" inputs are `block_number`
  and `chain_id`, both opaque integers from the request.

## What breaks (and what conspicuously does NOT)
- **Does NOT break under clock jitter:** approval bytes, keygen attestation
  payload, exit signature root — all are pure functions of request content and
  on-disk keys. Antithesis can skew/step the VM clock arbitrarily and these
  outputs are unchanged. That is a genuine `Always` invariant worth pinning,
  because *a future "add an expiry / deadline" change would break it*, and the
  property would catch the regression immediately.
- **The cost of immunity (the bite):** because nothing reads `expiresAt`, an
  expired or revoked CVM session is accepted whenever `verify_session=true`
  (today it's false, so moot — but the property documents that flipping it on
  does NOT add time-based revocation). And because the exit epoch is frozen at
  0, the same exit signature is valid irrespective of when it is requested —
  combined with W10 (no auth on sign-exit) and replay-ability (W2), time gives
  no natural expiry to a leaked exit-signing capability.

## Antithesis angle
- **`Always`** (SUT-side, around keygen attest payload + custody approval + exit
  root): *"the signed digest is independent of wall-clock time"* — operationally:
  with the system clock stepped by Antithesis between two otherwise-identical
  requests, the produced digests are byte-identical. (Pairs with W7-4's replay
  anchor; this variant explicitly varies *only* the clock.)
- **`Unreachable`** (SUT-side, defensive): *"any guardian-route code path reads
  SystemTime/Instant"* — assert-unreachable in a shim; if it ever fires, a
  time dependence was introduced (e.g. an added timeout or expiry) and the
  immunity property must be revisited. Documents the current contract.
- **`Reachable`**: *"verify_session_evidence accepts a session whose on-chain
  `expiresAt` is in the past"* (when verify_session is exercised) — the negative
  witness that time-blindness == no revocation.
- Antithesis lever: clock-skew/clock-jitter is a first-class Antithesis fault;
  this property tells the platform that tripping it should change *nothing* on
  the happy path — so any divergence is a real bug, not flakiness.

## Why the other six lenses miss it
- All six lenses look for where logic *exists*; this is a property about logic
  that *conspicuously does not exist* — a negative-space invariant ("no time
  dependence anywhere"). Failure Recovery (focus 3) covers "dependency hangs"
  but frames it as a missing-timeout availability bug, not as a clock-jitter
  *immunity* invariant that doubles as a regression tripwire.
- The session-expiry gap (F10) is owned by Security Boundaries as a *missing
  check*; the novel cut is reframing the entire binary's time-blindness as one
  cross-cutting `Always`/`Unreachable` pair that simultaneously (a) certifies
  clock-jitter robustness and (b) localizes the single ignored time field.

## Open questions (and why they matter)
- Is the hardcoded exit epoch `0` semantically correct for the beacon chain, or
  should it be the current epoch? → If it must be current-epoch, the exit
  message is malformed/rejected by the beacon node and the eject path is broken
  independent of everything else (liveness). *Matters because* it could be a
  total exit-path failure hiding behind a "looks deterministic" property.
- Will `verify_session` ever be enabled with expiry enforcement expected? → If
  yes, the immunity `Always` must be scoped to exclude the (future) expiry read.

## Instrumentation: MISSING
Clock-varied replay comparison is workload-driven; the `Unreachable` time-source
tripwire needs a one-line SDK guard. No SDK present.

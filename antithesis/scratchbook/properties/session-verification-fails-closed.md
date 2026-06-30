# sec-session-verify-fail-closed — When attestation IS verified, it must fail closed and check liveness

## Origin
Focus (6) Security Boundaries; SUT-analysis F10, F4, G3; lens lead F10.

## Files / functions / lines
- `src/enclave/guardian/mod.rs:115-225` `verify_session_evidence` (only runs when
  `verify_session == true`):
  - `:184-196` calls `verify_session_signature` (eth_call); `:194-196` bail if it
    returns false. Errors propagate via `?` -> handler 500 (fail-closed).
  - `:200-221` workload check is **skipped entirely when `workload_id.is_empty()`**
    (reef sends `""`). When present, it only compares `session.workloadId`.
  - **Never reads `isActive`, `expiresAt`, or calls `isSessionActive`** — the
    `CVMSession` struct exposes them (`src/io/session_registry.rs:30-33,45`) but
    `verify_session_evidence` ignores them.
- `src/io/session_registry.rs:51-101` `verify_session_signature` / `get_session` —
  rebuild a fresh provider per call; any RPC/parse error -> `.context(...)?` -> 500.

## Precise trust assumption
*When* the guardian does verify session evidence, the verification must (a) fail
closed on any error/false (never emit an approval), and (b) actually establish the
session is currently valid — i.e. active and not expired — not merely that the
signature once verified. Today (a) holds but (b) does NOT: an expired or revoked
session whose signature still verifies would be accepted.

## Adversarial sequence that violates it
1. (b)-violation: With `verify_session=true`, present a session whose signature is
   valid but whose on-chain record has `isActive=false` or `expiresAt < now`.
   `verify_session_evidence` returns Ok (it never checks those fields) -> approval
   emitted for a revoked/expired attestation.
2. workload bypass: send `workload_id=""` with `verify_session=true` -> the workload
   binding (`:200`) is skipped, so the session need not match the expected workload at
   all — the attestation is to *some* registered workload, not the intended one.
3. (a)-probe: inject network/RPC faults (Antithesis) so the eth_call errors; assert
   the guardian returns an error and NEVER an approval (must be fail-closed).

## Real-world impact
If/when attestation is turned on, expired/revoked sessions or wrong-workload sessions
would still yield custody approvals — defeating the purpose of attestation. (a) being
fail-closed is the redeeming property; (b)/workload-skip are the live gaps in the
verify path.

## Antithesis angle
- Fail-closed guard (true invariant, in the verify branch):
  `Always("verify_session=true path emits approval only if verify_session_evidence returned Ok")`
  combined with network-fault injection on the SessionRegistry RPC.
- Liveness gap (documents the bug, needs human confirm it's a bug):
  `Reachable("session accepted with verify_session=true but isActive/expiresAt never checked")`
  — fires whenever the verify branch passes, proving the liveness check is absent.
- Workload bypass: `Reachable("verify_session=true with empty workload_id ⇒ workload binding skipped")`.

## Instrumentation
**MISSING.** Fail-closed assertion can wrap the verify branch in
`verify_and_sign_custody_received`. The liveness-gap / workload-skip assertions are
SUT-side markers inside `verify_session_evidence`. SDK not yet a dependency. Note:
the verify path requires `SESSION_REGISTRY_*` env + a SessionRegistry contract; the
committed env file sets neither, so the workload/harness must provide a mock RPC to
exercise this at all (otherwise the branch is unreachable in test).

## Open questions (why they matter)
- **Is `verify_session` intended to be on?** (OQ#3) If no, this whole path is dead and
  F10 is moot; if yes, the missing `isActive`/`expiresAt` check is a live bug.
- **Should empty `workload_id` be allowed when `verify_session=true`?** reef sends
  both `verify_session=false` and `workload_id=""`, so the combination is untested —
  is skipping the workload binding intended? **Needs human input.**
- **What RPC does the deployment point `SESSION_REGISTRY_RPC_URL` at**, and does it
  agree with the on-chain contract (G3 doc drift)? Affects whether fail-closed is the
  realistic outcome.

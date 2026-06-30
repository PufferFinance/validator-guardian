# sec-cvm-stub-no-attestation — CVM_AGENT_STUB silently disables attestation and returns success

## Origin
Focus (6) Security Boundaries; SUT-analysis W11, F3; lens lead W11.

## Files / functions / lines
- `src/io/remote_attestation.rs:30-43` `AttestationEvidence::new`:
  - `:31-34` `if env CVM_AGENT_STUB == "true" { return Ok(AttestationEvidence::default()) }`
    — returns **empty default evidence** (empty session_id/signature/keys),
    short-circuiting the real CVM-agent call. **No production guard, no compile-time
    gate.**
- `src/enclave/guardian/mod.rs:21-58` `attest_new_eth_key_with_blockhash` -> `:56`
  `AttestationEvidence::new(&payload)` — keygen returns this evidence.
- `src/enclave/guardian/handlers/attest_fresh_eth_key_with_blockhash.rs:14-17` — returns
  **HTTP 201 CREATED** with the (stub) evidence; caller sees success.
- `container/guardian/env` — sets only `RUST_LOG`, `GUARDIAN_PORT`. `CVM_AGENT_STUB`
  is **not set** by default (good), but nothing in the binary prevents it being set in
  a deployed image.

## Precise trust assumption
The keygen endpoint claims to *produce attestation* (the enclave-ETH-key rotation is
supposed to be signed by the real CVM session key, later verified on-chain via
SessionRegistry). The trust assumption: a 201 from keygen means a genuine,
CVM-agent-signed attestation was produced. `CVM_AGENT_STUB=true` breaks this — a 201
can mean "empty, unsigned, default evidence."

## Adversarial sequence that violates it
1. A deployed/misconfigured image has `CVM_AGENT_STUB=true` (env, leftover from
   dev/CI). 2. keygen returns 201 with empty `AttestationEvidence::default()`.
3. Downstream consumers / on-chain verifiers that *would* check the attestation
   signature get an empty signature -> either it fails on-chain (liveness) or, if the
   verify path is off (F1), nobody notices the enclave key was never attested. The
   guardian *looks* healthy (201) while its root-of-trust output is vacuous.

## Real-world impact
Silent loss of the attestation root of trust for the enclave ETH key. Because the
guardian's whole purpose is attested key custody, shipping a stubbed build collapses
the security model while presenting a green/healthy surface (201). Especially
dangerous given there is no startup log/metric distinguishing stubbed from real.

## Antithesis angle
- `Reachable("keygen returned 201 with stub (empty) attestation evidence because CVM_AGENT_STUB=true")`
  at `remote_attestation.rs:31-34` — confirms the stub branch is reachable and that a
  success status can carry empty evidence. (This is the honest, always-true-when-set
  framing.)
- Defensive invariant the system *should* have (mark for human):
  `Unreachable("production-mode keygen returned empty attestation signature")` —
  would require a notion of "production mode" the code does not have today; flag as a
  recommended guard (refuse to start, or refuse 201, when stub is set outside dev).
- Workload note: this is config-driven, not request-driven — the workload sets the env
  to exercise both branches; faults are not central here.

## Instrumentation
**MISSING.** Needs a SUT-side `Reachable` in `AttestationEvidence::new` stub branch and
ideally a startup assertion. SDK not yet a dependency.

## Open questions (why they matter)
- **Can `CVM_AGENT_STUB=true` reach a production image?** (OQ#5) If the build/deploy
  pipeline doesn't strip or forbid it, attestation can be silently disabled. **Needs
  build audit / human input.** This decides whether the property is a real production
  risk or only a test-harness convenience.
- Should the binary refuse to start (or refuse to 201) when the stub is set and a
  "production" flag is present? Design recommendation, needs owner sign-off.

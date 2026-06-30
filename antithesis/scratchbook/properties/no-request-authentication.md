# sec-no-request-authn — Every privileged endpoint is reachable with no authentication

## Origin
Focus (6) Security Boundaries; SUT-analysis §2 ("Middleware: none"), §5 trust-boundary reframe.

## Files / functions / lines
- `src/bin/guardian.rs:35-71` — the axum Router has **no tower layers**: no auth
  middleware, no TLS, no body-size limit, no timeout, no `CatchPanicLayer`. Routes:
  `/upcheck` (GET), `/eth/v1/keygen` (POST/GET), `/guardian/v1/validate-custody` (POST),
  `/guardian/v1/sign-exit` (POST) — all `.with_state(app_state)` only.
- `:67` binds `0.0.0.0:PORT`. `:69` `_ = axum::Server::bind(...)...` — bind result
  discarded (silent exit on bind failure, not in scope here but relevant to surface).
- Handlers (`handlers/*.rs`) deserialize JSON and call core logic directly — no
  per-handler caller check anywhere.

## Precise trust assumption
The two privileged operations — minting a custody approval (validate-custody) and
minting an exit signature (sign-exit) — are assumed to be invoked **only** by the
trusted reef/BFF orchestrator over a private network link. There is no in-process
authentication or authorization of the caller; trust is delegated entirely to
network topology.

## Adversarial sequence that violates it
Any actor with network reachability to the port can invoke every endpoint with no
credential. Combined with: F1 (validate-custody emits approvals with
`verify_session=false`) and W10 (sign-exit has zero authz), reachability == full
capability. The only barrier is the assumed-private network.

## Real-world impact
If the port is reachable beyond the trusted orchestrator (misconfigured firewall,
shared network, SSRF from a co-located service), an attacker directly drives custody
approval and forced-exit signing. The entire security model rests on an external,
unverified-in-code network assumption.

## Antithesis angle
- This is primarily a *topology / deployment* property, not a code invariant, so the
  honest assertion is a documentation one:
  `Reachable("privileged endpoint served a request with no authentication present")`
  — true on every call, confirming there is no authn layer to regress against.
- More useful as a **gating fact** for the other properties: it establishes that
  F1/W10/W1/W3 are remotely exploitable *iff* the port is reachable, so the workload
  should treat "unauthenticated caller" as the default threat model.
- Workload-driven (the workload simply IS an unauthenticated caller). No fault needed.

## Instrumentation
**MISSING**, and arguably low-value to instrument as an SDK assertion (it would always
fire). Better recorded as a documented trust assumption + an open question for the
deployment owner. Included for completeness of the Security-Boundaries lens.

## Open questions (why they matter)
- **Is validate-custody / sign-exit reachable by untrusted clients in the TDX network
  topology, or only by reef over a private link?** (catalog OQ#4) This single answer
  determines whether F1/W10/W1/W3 are remotely exploitable vulnerabilities or
  defense-in-depth gaps. **Needs human input — highest-leverage question for this
  lens.**
- Is application-level authn (e.g. mTLS, signed requests from reef) intended to be
  added, or is network isolation the permanent control?

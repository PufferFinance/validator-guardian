# sec-approval-implies-session-verified — Custody approval must not be emitted without attestation/session verification

## Origin
Focus (6) Security Boundaries; SUT-analysis F1/G1, W1; lens lead F1.

## Files / functions / lines
- `src/enclave/guardian/mod.rs:60-104` `verify_and_sign_custody_received` — the request handler core.
  - `:71-73` `if request.verify_session { verify_session_evidence(...).await?; }` — the **entire** on-chain session check is gated by the client-controlled bool.
  - `:88-95` `approve_custody(...)` always runs (signs the custody approval), regardless of `verify_session`.
- `src/enclave/types.rs:89` `pub verify_session: bool` — client-controlled field in `ValidateCustodyRequest`.
- `src/enclave/guardian/mod.rs:115-225` `verify_session_evidence` — does the SessionRegistry `eth_call`.
- Consumer: `/home/fawad/puffer/projects/reef/reef-guardian/src/handlers/api/webhooks/provision_or_skip/new_registration.rs:266` sets `verify_session: false`, `:265` `workload_id: ""`, `:257-259` empty `session_id`/`attestation_signature`/`session_public_key`.

## Precise trust assumption
The system's claimed root of trust (README, migration-guide) is that the guardian
verifies the validator enclave's CVM **session/attestation evidence on-chain**
before co-signing a custody approval. The honest invariant a security reviewer
would want: *a custody approval signature is emitted only for a payload whose
attestation was actually verified.* In code, "verified" == `verify_session_evidence`
ran and returned Ok.

## Adversarial sequence that violates it
1. Attacker (any party that can reach the port — there is NO authn/authz middleware,
   `bin/guardian.rs` has no tower layers) crafts a `ValidateCustodyRequest` with a
   well-formed BLS payload (passes S1-S6) but with `verify_session: false` and empty
   session fields.
2. `verify_and_sign_custody_received` skips `verify_session_evidence` entirely
   (`:71`), runs only the cryptographic well-formedness checks, persists the share,
   and returns a valid `enclave_signature`.
3. The approval is byte-identical to one produced with `verify_session: true`
   (W1: `approve_custody` commits to none of session_id/attestation/share index/
   guardian pubkey), so on-chain `GuardianModule` cannot tell the difference.

## Real-world impact
If reached on >= M guardians, an unattested (potentially adversary-chosen) validator
keyset gets a valid quorum of custody approvals -> provisioned/funded on-chain ->
un-ejectable / funds stuck. This is the single highest-impact attestation gap.

## Honest framing (CRITICAL — needs human judgment)
**Attestation is OFF in production today** — reef hardcodes `verify_session=false`
(confirmed in reef). So `Always(approval ⇒ session_verified)` would **fail
immediately** on the real workload and is NOT a true invariant of the system as
shipped. Two honest options:
- **(A) Document the risky branch as reachable** (recommended for the current code):
  `Reachable("custody approval emitted with verify_session=false (attestation NOT verified)")`
  placed where the signature is returned with `request.verify_session == false`.
  This makes Antithesis *confirm* the unverified path is taken — the security fact
  the team must consciously accept or fix.
- **(B) Aspirational invariant** for if/when attestation is turned on:
  `Always(approval_emitted ⇒ session_verification_ran)` — only meaningful once
  reef flips the flag; would be the guardrail proving they can't regress.
Both should be instrumented. (A) is true today; (B) guards the intended future.

## Antithesis angle
Workload-driven: send custody requests with `verify_session` both true and false
and assert which branch was taken. Combine with the network/RPC faults so the
`verify_session=true` path is also exercised under partial failure (it must
fail-closed, never emit an approval if the eth_call errors — see related property
`sec-session-verify-fail-closed`).

## Instrumentation
**MISSING.** Needs SUT-side `Reachable`/`Always` at `mod.rs:88-96` distinguishing the
two branches (e.g. a bool `session_verified` set true only after
`verify_session_evidence` returns Ok). The SDK (`antithesis-sdk` crate) is not yet a
dependency.

## Open questions (why they matter)
- **Is `verify_session` intended to be flipped on?** (catalog OQ#3) If yes, (B) is a
  live guardrail and reef's `false` is a bug; if no, the whole SessionRegistry path is
  dead code and the attestation story is aspirational. Determines whether this is a
  bug or by-design. **Needs product decision.**
- **Is validate-custody reachable by untrusted clients** (catalog OQ#4)? Decides
  whether the bypass is remotely exploitable or defense-in-depth.


---

> **[merged]** consolidated from discovery focus file `prop-focus-7/w7-6-no-signing-capability-without-verification-invariant.md`

# W7-6 — The unstated master invariant: "no signing capability is conferred without prior verification" (and the single combination that violates it)

## Origin / intuition
The prompt asked for *one cross-cutting invariant that captures "no signing
capability without prior verification."* The guardian's entire reason to exist
is to be a gate: a BLS share must not become a usable signing capability unless
its custody was verified. But the capability (a persisted share that sign-exit
will sign with) is conferred by a **specific combination of states**, no single
one of which any lens owns end-to-end:

> (attestation OFF) AND (a share was persisted) AND (sign-exit is open) ⇒
> an **unattested** keyshare is now a **live exit-signing capability**.

This is the system-level invariant that ties together SUT G1 (attestation off),
§3 write-before-success persistence, and W10 (sign-exit unauthenticated). The
novelty is stating it as ONE invariant over the *lifecycle of a capability*,
not as three separate local findings.

## The capability chain (files / functions / line numbers)
1. **Verification is conditional and currently inert.**
   `verify_and_sign_custody_received` gates the only attestation check on the
   client flag: `if request.verify_session { verify_session_evidence(...) }`
   (`mod.rs:71`). Reef sends `verify_session: false`
   (`reef/.../new_registration.rs:266`). So the "prior verification" half of the
   invariant is, in production, **never executed** — what remains is S1-S6
   (cryptographic well-formedness), which says nothing about *provenance*.
2. **The capability is persisted BEFORE the approval is even produced.**
   `mod.rs:82-85` writes the BLS share to disk, THEN `mod.rs:88` computes the
   approval. So the share becomes a sign-exit-usable capability the instant
   write_bls_key returns — even if approval, or the whole request, later fails
   or the process crashes (SUT F9 orphaned secret).
3. **sign-exit confers signing with NO gate.** `sign_voluntary_exit_message`
   (`mod.rs:352-370`) reads any persisted share by caller-supplied
   `(bls_pub_key_set, guardian_index)` and signs — no custody check, no
   attestation check, no ownership proof (W10). epoch hardcoded 0.
4. **Net:** a share that was persisted (step 2) with attestation skipped
   (step 1) is signable by an unauthenticated caller (step 3). The gate the
   guardian exists to enforce is, end-to-end, **not enforced**.

## What breaks
- The product invariant "a validator can only be force-exited if M guardians
  each verified custody of a share" degrades to "M guardians each persisted a
  well-formed share and will sign an exit for anyone who asks." With attestation
  off, *verification of provenance* is absent from the capability's entire
  lifecycle. Reaching M unauthenticated sign-exit calls force-ejects a live
  validator (SUT §13(d)) — the catastrophic systematic case.
- Even the *intended* safety reduces to "the share decrypts under my enclave
  key" — but that share was handed to the guardian by the **unauthenticated
  reef/BFF orchestrator** (SUT trust-boundary reframe §5), so the root of trust
  is an unauthenticated caller, not attestation.

## Antithesis angle
This is best expressed as a **lifecycle invariant** spanning two endpoints —
ideally SUT-side, tracking per-share state:
- **`Always`** (SUT-side): *"every share that sign-exit signs with was persisted
  by a validate-custody call whose verification step actually ran"* — i.e.
  sign-exit must not be reachable for a share whose custody had
  `verify_session=false`. Implement by tagging each persisted share with whether
  verification ran (a metadata bit at write time) and asserting it at sign-exit.
  **This is the master assertion; it will FAIL today** (by design it documents
  the gap), so it is most useful as a `Reachable`/`Sometimes` witness first:
- **`Reachable`**: *"sign-exit produces a valid exit-signature share for a
  validator whose custody was approved with `verify_session=false`"* — the
  concrete witness that an unattested share became an exit capability.
- **`Sometimes`**: *"a BLS share exists on disk that was persisted by a
  validate-custody request which then did NOT return a successful approval"* —
  the orphaned-capability state (write-before-success), provable by crashing
  after `mod.rs:85` but before `mod.rs:95`.
- **`AlwaysOrUnreachable`** (forward-looking sentinel): *"if attestation is ON
  (verify_session true), no share is persisted before verify_session_evidence
  returns Ok"* — pins the intended ordering for the day attestation is enabled.

## Why the other six lenses miss it
- Each component finding is owned by a different lens — G1/attestation-off
  (Protocol/Security), write-before-success (Failure Recovery), W10/sign-exit
  no-auth (Security) — but **no lens owns the conjunction across the share's
  lifecycle**. The six are organized by *failure category*; this invariant is
  organized by *the lifetime of a single capability* and only emerges when you
  trace one share from custody-persist to exit-sign. That cross-endpoint,
  cross-lens stitch is exactly the wildcard's remit ("a property that only
  matters under a specific combination").

## Open questions (and why they matter)
- Is sign-exit reachable by untrusted clients in the TDX topology, or only by
  reef over a private link? (SUT open Q4) → Decides whether the invariant
  violation is remotely exploitable or defense-in-depth. *Matters because* it
  sets the severity from "catastrophic" to "hardening."
- Is `verify_session` intended to be flipped ON? (SUT open Q3) → If yes, the
  `Always` master invariant becomes a true production guarantee to enforce; if
  no, the entire verification half is aspirational and the invariant can never
  hold.

## Instrumentation: MISSING (and necessarily SUT-side)
Cannot be expressed from the workload alone — requires tagging each persisted
share with "was it verified?" at custody-write time and reading that tag at
sign-exit time. No SDK present; this is the highest-value SUT-side instrumentation
target in the catalog.

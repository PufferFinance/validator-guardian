# Property: approval-binds-distinguishing-request-fields

Focus: (9) Idempotency & Replay (binding / replay-equivalence)
Slug / canonical ID: `approval-binds-distinguishing-request-fields`

## One-sentence property
Two `validate-custody` requests that differ in any field the system treats as
security-relevant must produce **different** `enclave_signature`s; conversely the
signature is a pure function of exactly `(guardian_module_address, chain_id,
validator_index, blsPubKey, withdrawalCredentials, depositSignature, depositDataRoot)`
and of **nothing else** — in particular it is byte-identical for two requests that
differ only in `verify_session`, `workload_id`, `session_id`,
`attestation_signature`, or `session_public_key`.

## What led to this property
- W1 (SUT §9): "approval bound to neither the attestation nor the share/guardian.
  `approve_custody` commits only to group-level deposit fields — **not** `session_id`,
  **not** share index `i`, **not** the enclave's own pubkey. Two requests differing
  only in `verify_session` produce **byte-identical** signatures."
- W2/W3 (SUT §9): no nonce/deadline/used-signature replay guard on-chain;
  `validator_index` is attacker-chosen and `verify_custody` never references it.
- Focus prompt: the custody approval commits to S6 fields but NOT to `session_id`,
  share index, or the guardian's own pubkey.

This is the **replay-equivalence** property: it makes the (currently true and
arguably dangerous) behavior *explicit and observable*, so Antithesis can both
confirm it and flag if a fix later changes the binding.

## Code evidence (files + functions + lines)
- Preimage construction: `src/enclave/guardian/mod.rs:322-336` `approve_custody`. The
  ABI-encoded tuple is exactly:
  `[Address(guardian_module_address), Uint(chain_id), Uint(validator_index),
    Bytes(pk_set.public_key()), Bytes(withdrawal_credentials),
    Bytes(signature), FixedBytes(deposit_data_root)]`.
  → No `session_id`, no share index `i`, no enclave pubkey, no `verify_session`, no
  `workload_id` in the preimage.
- The session/attestation fields are consumed (if at all) only by
  `verify_session_evidence` (`mod.rs:115-225`), which is gated on
  `request.verify_session` (`mod.rs:71`) and returns `()` — it never feeds the
  approval preimage.
- reef sends `verify_session: false` and empty session fields
  (`reef-guardian/.../new_registration.rs` `generate_validate_custody_request`), and
  `validator_index: payload.puffer_module_index` (caller-derived).
- On-chain replay guard: only the monotonic `nextToBeProvisioned` is checked
  (reef `provision_validator.rs` "next validator" gate, ~line 57-80); there is no
  nonce/used-sig mapping (SUT §9 W2).

## What breaks / what this documents
- **Replay equivalence (the risk):** because the signature is independent of
  `verify_session` and the session fields, an approval obtained with attestation OFF
  is **indistinguishable on-chain** from one obtained with attestation ON. If
  attestation is ever turned on as the security story (Open Q3, SUT §13), this
  property shows the on-chain consumer still cannot tell the difference → the
  attestation guarantee does not propagate to the signature. The signature also binds
  an **attacker-chosen `validator_index`** (W3) and carries **no nonce/deadline**
  (W2) → the same approval is replay-valid for as long as the preimage fields stay
  valid.
- **As a regression anchor:** if a future change *intends* to bind `session_id` or
  the guardian pubkey into the preimage (to fix W1), this property's
  "identical-under-verify_session-toggle" assertion will (correctly) start failing —
  signalling the binding changed. That is the desired behavior: the property is the
  tripwire.

## Suggested assertion(s) and types
1. **`Always`** (workload-side): send two requests identical except
   `verify_session` toggled (and session fields populated vs empty) with the same
   enclave key; assert the two `enclave_signature`s are **byte-equal**. This pins the
   *current* W1 reality and turns any future binding change into a visible event.
   Type rationale: documents an invariant that holds today on every such pair.
   **Instrumentation: workload-only; no SUT change.** Mark as documenting a *known
   weakness*, not a passing safety guarantee.
2. **`Always`** (workload-side): send two requests differing **only** in
   `validator_index`; assert the signatures **differ** (validator_index IS in the
   preimage). Confirms the preimage actually depends on the field it claims to.
   **Instrumentation: workload-only.**
3. **`Reachable`** / SUT-side marker on the branch
   `verify_session == false` in `verify_and_sign_custody_received` (`mod.rs:71`):
   record that an approval was produced **without** any attestation/session check —
   makes the "attestation OFF" path a first-class, searchable outcome. Message:
   "custody approval signed with verify_session=false (no on-chain session check)".
   **Instrumentation: MISSING.**
4. **`Unreachable`** candidate (forward-looking, only if Open Q3 resolves to "session
   verification must gate signing"): assert it is impossible to reach the
   `write_bls_key` / `approve_custody` block with `verify_session == false` in a
   production build. Today this WOULD fire immediately (reef sends false), so it is
   **not** appropriate now — note it as the assertion to add *if/when* product
   decides attestation is mandatory. **Instrumentation: MISSING (intentionally
   deferred).**

## Antithesis angle (faults / timing / interleaving)
- Input-variation driven: the workload generates request pairs differing in exactly
  one field and compares outputs — Antithesis explores the field space and the
  assertion captures the binding relation. Faults are secondary here; the value is in
  *systematic field-toggling* the existing tests never do (they hardcode
  `verify_session=false`, SUT §12).
- Network partition / asymmetric bad-node between reef and one guardian, combined with
  no on-chain nonce, is the real-world replay scenario this property contextualizes:
  a captured-and-replayed approval stays valid.

## Timing / config dependencies
- None timing-sensitive; signature is time-independent (see
  `custody-approval-deterministic-under-retry`).
- Property (3)/(4) semantics depend on **Open Q3** (is `verify_session` meant to be
  flipped on?) and **Open Q4** (is the endpoint reachable by untrusted clients?).

## Open questions
- **Open Q3 (SUT §13): is `verify_session` intended to become mandatory?** *Why it
  matters:* decides whether assertion (4) (`Unreachable` on verify_session=false
  signing) is a future safety invariant or permanently inapplicable. *What changes:*
  if yes, W1's "signature identical regardless of attestation" becomes a real
  security defect (the on-chain consumer can't distinguish attested vs unattested) and
  this property jumps to High; if no, the whole session path is dead code and (1) just
  documents inert behavior. **Needs human/product input.**
- **Open Q4 (SUT §13): is `/guardian/v1/validate-custody` reachable by untrusted
  clients?** *Why it matters:* if yes, W3 (attacker-chosen `validator_index` with no
  nonce) is remotely exploitable — an attacker can mint approvals binding arbitrary
  validator indices. *What changes:* elevates (2) from a binding sanity check to a
  genuine attack-surface property. **Needs human input on TDX network topology.**
- **Does `GuardianModule` on the target contract branch even verify this 7-field
  preimage?** (W0, SUT §9, Open Q1) *Why it matters:* if the deployed contract still
  verifies the old 5-field preimage, every approval fails on-chain regardless — the
  binding question is moot until preimage drift is resolved. *What changes:* gates
  whether any of these signatures are accepted end-to-end. **Needs human input.**


---

> **[merged]** consolidated from discovery focus file `prop-focus-5/approval-binds-validator-index.md`

# sec-approval-not-bound-to-share — Custody approval is not bound to attestation, share, guardian identity, or a nonce

## Origin
Focus (6) Security Boundaries; SUT-analysis W1, W2, W3; lens leads W1/W3.

## Files / functions / lines
- `src/enclave/guardian/mod.rs:312-350` `approve_custody` — the signed preimage
  (`:322-336`) commits to, in fixed ABI order:
  `(guardian_module_address, chain_id, validator_index, blsPubKey, withdrawalCredentials, depositSignature, depositDataRoot)`.
  It does **NOT** commit to: `session_id`, `attestation_signature`, the share index `i`
  that decrypted, the enclave's own pubkey, any nonce/deadline, or `verify_session`.
- `validator_index` flows in unchecked from `request.validator_index`
  (`src/enclave/types.rs:90`) -> `verify_and_sign_custody_received` (`mod.rs:92`) ->
  `approve_custody` (`mod.rs:325`). `verify_custody` (`:281-310`) never references
  `validator_index` — it is bound to *nothing else* in the payload.

## Precise trust assumption
A guardian's custody approval should attest "this specific guardian verified custody
of this specific share for this specific validator under verified attestation." The
on-chain `GuardianModule` consumes it as such. The reality: the signature binds only
group-level deposit fields + a caller-chosen `validator_index`, with no link to the
attestation that (maybe) ran, to which guardian signed, or to which share decrypted —
and there is no replay/nonce protection (confirmed no on-chain used-signature map;
only monotonic `nextToBeProvisioned`).

## Adversarial sequences
- W3 (validator_index rebinding): obtain an approval that binds an arbitrary
  `validator_index` to a legitimate keyset — the guardian signs whatever index the
  caller supplies. Two requests differing only in `validator_index` both succeed.
- W1 (attestation-independence): two requests differing ONLY in `verify_session`
  (true vs false) produce **byte-identical** signatures — the approval cannot encode
  whether attestation happened. So even if attestation is turned on, the signature is
  not evidence that it ran.
- W2 (replay): the same approval can be replayed; nothing in the signed bytes is
  single-use.

## Real-world impact
The custody approval is a weaker statement than it appears. An attacker who can reach
the endpoint can get approvals binding chosen validator indices, and the signature
gives the on-chain verifier no cryptographic evidence that attestation occurred or
which guardian/share it pertained to. Defense-in-depth is entirely on the
(unauthenticated) reef/BFF orchestrator.

## Antithesis angle
- `Reachable("two custody approvals binding the same keyset to different validator_index values were both produced")`
  — confirms W3 (validator_index is attacker-steerable, bound to nothing).
- `Always("approval signature is independent of verify_session")` framed as the
  *observation* W1: assert that for fixed payload, toggling `verify_session` yields the
  same signature bytes -> `Reachable("identical approval signature produced with and without attestation verification")`.
  This is a true, demonstrable fact (documents the gap), not a crash.
- Mostly workload-driven (adversarial request shaping). These document missing
  bindings the system arguably *should* have; mark for human judgment (some may be
  intentionally delegated to on-chain logic).

## Instrumentation
**MISSING.** SUT-side: record the signed-preimage field set and the resulting
signature in `approve_custody`, plus a flag for whether the verify branch ran, so the
workload can assert the independence/rebinding facts. SDK not yet a dependency.

## Open questions (why they matter)
- **Is the absence of session/share/guardian/nonce binding intentional** (because the
  on-chain `GuardianModule` + reef enforce ordering and uniqueness)? Decides
  defense-in-depth vs bug. **Needs human input.**
- **Does on-chain logic bind `validator_index` to the keyset elsewhere** (e.g. via
  `nextToBeProvisioned`)? If yes, W3 is mitigated on-chain; if not, it is a real
  rebinding vector. **Needs contracts review (cross-repo).**
- Related to OQ#1 (preimage drift W0): the approve preimage is already known to have
  been wrong once (`61bf1d2`); confirm the current 7-field order matches the verifier.

## Investigation Log

#### Does `LocalWallet::sign_message` emit canonical low-S (matters for on-chain acceptance / replay framing)?
- **Examined:** `Cargo.lock` (ethers 2.0.14, k256 0.13.4, ecdsa 0.16.9);
  `ethers-signers-2.0.14/src/wallet/mod.rs:85-160`; `ecdsa-0.16.9/src/recovery.rs:218-227`
  + `hazmat.rs:93-112`; `k256-0.13.4/src/ecdsa.rs:182-208`. (Full trace recorded in
  `approval-deterministic-and-idempotent.md` Investigation Log.)
- **Found:** signature `s` is normalized to low-S at `k256-0.13.4/src/ecdsa.rs:194`
  (`sig.normalize_s()`); high-S is rejected on verify (`:203-205`). Nonce is RFC-6979
  deterministic with empty `ad` → the same approval bytes are reproduced on every replay.
- **Not found:** no high-S emission path; no nonce/deadline/used-sig field in the
  preimage (confirms the existing W1/W2/W3 binding-gap analysis — unchanged).
- **Conclusion:** **RESOLVED (low-S sub-point).** The signature is on-chain-acceptable
  (canonical low-S), so the *replay-equivalence* risk this property documents is NOT
  mitigated by any signature-malleability/normalization quirk — a captured approval is
  byte-stable and replay-valid for as long as the preimage fields stay valid. This
  *reinforces* the replay-resistance concern (no malleability noise to muddy a replay).
  The binding/`verify_session`/`validator_index` open questions (Q3/Q4/W0) are
  unaffected and remain **needs human input**.

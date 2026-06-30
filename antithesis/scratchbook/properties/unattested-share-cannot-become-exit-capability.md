# unattested-share-cannot-become-exit-capability

Added during the evaluation pass (gap G2 — wildcard F-FIND-6). The "catastrophe
chain" was described in `property-relationships.md` as the highest-value
end-to-end Antithesis timeline, but had no first-class catalog row — so the single
best end-to-end scenario risked never being built. This property names it.

## What led to this property

Three independently-cataloged facts compose into one end-to-end failure, and no
single property exercises the *composition*:

1. **Attestation is bypassable / off** — `verify_session` is client-controlled and
   reef sends `false` (precondition P1; [[attestation-verified-before-approval]]).
   So a share can be accepted with no attestation verification.
2. **The share is persisted before (and regardless of) success** — `verify_and_
   sign_custody_received` `write_bls_key`s the decrypted share
   (`src/enclave/guardian/mod.rs:82-85`) before `approve_custody`, with no
   rollback ([[no-orphaned-unacknowledged-secret]]).
3. **`sign-exit` is ungated** — it mints an exit-signature share for any stored
   share with no authorization, attestation, or custody check, epoch hardcoded 0
   ([[sign-exit-requires-authorization]], `mod.rs:352-394`).

Therefore: an **unattested (or orphaned) BLS share becomes a live voluntary-exit
signing capability**. Across ≥M guardians this can force-eject live validators.
This is the master invariant w7-6 ("no signing capability without prior
verification") expressed as a concrete, testable timeline rather than three
disconnected properties.

## What goes wrong if violated

The guardian produces a valid exit-signature share for a validator whose custody
was never attested (or whose share was orphaned by a crash). With a threshold of
such shares, a live, funded validator is force-exited — direct loss of liveness
and potential funds disruption for the validator operator (SUT analysis §13d).

## Antithesis angle

This is a genuine **end-to-end timeline** property — exactly Antithesis's strength
(it composes a request sequence with a fault):

1. Workload submits `validate-custody` with `verify_session=false` (or, with
   node-termination, crash the guardian after `write_bls_key` but before the
   approval response — producing an *orphaned* share).
2. Workload then calls `sign-exit` for that share's `(bls_pub_key_set,
   guardian_index, validator_index)`.
3. Assert the dangerous outcome: an exit signature was produced for a share that
   was never attested / never acknowledged.

## Suggested instrumentation (SUT-side — MISSING)

The clean encoding needs a per-share **provenance marker** written alongside the
key at custody time: "was `verify_session` true and did `verify_session_evidence`
pass?" Then at `sign-exit`:

- `Reachable("exit signature produced for a share whose custody had
  verify_session=false")` — witnesses the live gap today.
- Aspirational `Always("sign-exit ⇒ the share's custody verification ran")` — the
  guardrail once attestation is mandatory and an authorization gate exists.
- `Sometimes("sign-exit signed an orphaned share with no recorded approval")` —
  the crash-composed variant (requires node-termination).

The marker does not exist today (no metadata is stored beside the plaintext-hex
key, `io/key_management.rs`) — adding it is the key instrumentation task and is
shared with [[no-orphaned-unacknowledged-secret]].

## Open Questions

- Is sign-exit's lack of authorization / the unattested-share acceptance **by
  design** (the guardian trusts reef over a private link)? `(needs human input —
  if by design, this property documents an accepted risk; if not, it is a live
  critical bug. Cross-refs catalog Q4/Q6.)`
- Is the guardian's port reachable by anything other than the trusted reef
  instance? `(needs human input — gates whether this is remotely triggerable.)`
- Dominance: this property subsumes parts of [[attestation-verified-before-approval]],
  [[no-orphaned-unacknowledged-secret]], and [[sign-exit-requires-authorization]];
  if it is built as the end-to-end timeline, are the three component properties
  still needed individually, or do they become the unit-level decomposition?
  `(catalog-curation question — see evaluation Bias B1.)`

### Investigation Log

This property was added in the evaluation pass; it composes three already-confirmed
facts, so no new code investigation was needed.

#### Is the ungated/unattested-share acceptance by design?
- Examined: the three component findings are code-confirmed (`mod.rs:71` client-
  controlled gate; `:82-85` persist-before-approval; `:352-394` ungated sign-exit
  — see the Investigation Logs in [[sign-exit-requires-authorization]] and
  [[no-orphaned-unacknowledged-secret]]). What cannot be determined from code is
  *intent* — whether the guardian is meant to trust reef over a private link.
- Not found: any in-code authorization/attestation gate on sign-exit, or any
  reconciliation for orphaned shares.
- Conclusion: `(needs human input)` — accepted-risk vs critical bug is a
  product/threat-model decision (catalog Q4/Q6), not a code question.

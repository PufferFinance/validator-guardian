# custody-rejects-invalid-deposit-data

Synthesized during catalog assembly from enforced invariants S1–S3 (see
`sut-analysis.md` §6) — no single discovery focus owned it as a standalone
property, but every focus that read `verify_and_sign_custody_received` referenced
these checks as the fail-closed precondition for the approval signature.

## What led to this property

`verify_and_sign_custody_received` (`src/enclave/guardian/mod.rs:60-104`) must
reject a custody payload — emitting **no approval signature and persisting no
share** — unless the deposit data is cryptographically well-formed. Three checks
in `verify_deposit_message` (`mod.rs:260-279`) and `verify_custody`
(`mod.rs:281-310`) enforce this:

- **S1** — `pk_set.public_key().verify(signature, deposit_message_root)` must
  hold, else `bail!("DepositMessage signature invalid")` (`mod.rs:263-269`).
- **S2** — the recomputed `deposit_data_root` must equal the supplied
  `keygen_payload.deposit_data_root`, else `bail!("DepositDataRoot invalid")`
  (`mod.rs:271-277`).
- **S3** — `bls_pub_key` must be derivable from `bls_pub_key_set`
  (`verify_public_keys_match`, `mod.rs:285-288`).

The order matters: S1/S2 (`verify_deposit_message`) run **before** S4
(`verify_custody`) and before the share is written (`mod.rs:76` → `:79` → `:82`).

## What goes wrong if violated

A guardian that approved a payload with an invalid deposit signature or a
mismatched deposit-data-root would be co-signing custody for a validator whose
on-chain deposit data does not correspond to the BLS key the guardians hold →
the validator can be activated with deposit data the guardian set cannot honor,
or funds routed to the wrong withdrawal credentials. This is the core
"don't approve garbage" safety gate.

## Antithesis angle

This is primarily a **fail-closed safety invariant exercised by input-space
coverage**: the workload submits payloads with (a) valid deposit data, (b) a
corrupted `signature`, (c) a mismatched `deposit_data_root`, (d) a `bls_pub_key`
that doesn't match the set, and (e) a mismatched `withdrawal_credentials` /
`fork_version` (which changes the signing domain — see
[[fork-version-source-consistency]]). Assert that an approval is emitted **only**
in case (a).

**Honesty note for the evaluation pass:** S1/S2/S3 are pure functions of the
request payload and are largely **unit-testable in isolation** — Antithesis's
added value here is modest unless combined with fault injection (e.g. a torn/
restarted state, or the interplay with `fork_version` trust and the
`.zip`-length assumption). Priority is therefore **Medium**, and the property is
most valuable as the `Always`-side guardrail that the more interesting properties
(preimage match, share binding) build on.

## Suggested instrumentation

- `Always` ("custody approval emitted only for a payload that passed S1+S2+S3"):
  place an Antithesis `assert_always!` at the point an approval is about to be
  returned, with a flag tracking that all three checks ran and passed. **MISSING**
  (no SDK in repo; today these are `bail!` → HTTP 500, not assertions).
- `Sometimes` ("a payload was rejected for invalid deposit signature") and
  `Sometimes("rejected for deposit-data-root mismatch")` to confirm the workload
  actually exercises the negative paths. **MISSING.**

## Open Questions

- Does any consumer ever legitimately send a non-default `fork_version` in the
  custody payload, and if so does S1's `deposit_message_root` (which folds in
  `fork_version`) still match the on-chain expectation? Cross-refs
  [[fork-version-source-consistency]]. *(why it matters: if the domain is
  attacker/caller-chosen, S1 can pass for a deposit that is invalid under the
  network's real genesis fork version.)*

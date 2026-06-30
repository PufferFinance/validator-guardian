---
sut_path: /home/fawad/puffer/projects/validator-guardian
commit: f7dbd88d99de21a6d8caba5f7de8216624ab1098
updated: 2026-06-30
external_references:
  - path: /home/fawad/puffer/projects/reef
    why: pins consumer call patterns (verify_session=false, guardian_index=0) that drive several cross-property connections.
  - path: /home/fawad/puffer/projects/coral
    why: confirmed coral does not call the guardian; reframes the trust boundary.
---

# Property Relationships — `guardian` binary

Lightweight clustering of the 29 cataloged properties by shared code paths,
shared failure mechanisms, and suspected dominance. Every slug below appears in
`property-catalog.md`.

## Cluster 1 — Attestation enforcement (the gate the guardian exists to be)

- [[attestation-verified-before-approval]] *(master)*
- [[session-verification-fails-closed]]
- [[cvm-stub-never-in-production]]
- [[no-request-authentication]] *(threat-model precondition)*

**Shared mechanism:** whether, and how soundly, the validator-enclave attestation
is verified before the guardian co-signs. **Dominance:**
`attestation-verified-before-approval` is the umbrella invariant — if it held
(attestation always verified before any signing), `session-verification-fails-closed`
and `cvm-stub-never-in-production` become the two ways "verified" can still be
*unsound*, and `no-request-authentication` becomes moot. Today (precondition P1)
the umbrella is OFF, so all four are live and independent.

## Cluster 2 — Byte-exact agreement with external verifiers

- [[custody-preimage-matches-onchain-verifier]] *(critical)*
- [[rotate-key-preimage-matches-onchain]]
- [[keygen-evidence-accepted-onchain]] *(added in evaluation — the LIVE keygen twin)*
- [[attestation-payload-reconstruction-matches]]
- [[session-public-key-parse-no-wrong-identity]]
- [[pubkey-representation-consistent]]

**Shared mechanism:** the guardian hand-rolls byte layouts (three keccak/abi
preimages + key encodings) that must match an external implementation (Solidity
contract, Automata CVM signer, or reef's wire format) exactly. Any drift =
silent verification failure (liveness) or a forgery seam. They share the failure
*class* ("one byte off ⇒ silent reject"). `custody-preimage-matches-onchain-verifier`
is the highest-severity because drift there is **confirmed** and stalls
provisioning protocol-wide; `keygen-evidence-accepted-onchain` is its twin on the
keygen→`rotateGuardianKey` path that is **live on-chain today** (the custody path
is inert under P1, the keygen path is not).

**W0 dual-bind (evaluation R3):** `custody-preimage-matches-onchain-verifier` and
`rotate-key-preimage-matches-onchain` are an **unsatisfiable conjunction** — no
single deployed contract branch satisfies both (7-field custody only on
`fix/increase-guardian-signatures-security`; ROTATE_GUARDIAN_KEY only on
`feat/tdx`). Model them as a cross-process XOR, not independent oracles.

## Cluster 3 — Custody cryptographic well-formedness (the verify_custody gate)

- [[stored-share-matches-committed-pubkey-share]] *(strongest holdable invariant)*
- [[custody-rejects-invalid-deposit-data]]
- [[fork-version-source-consistency]]
- [[trial-decrypt-distinguishes-corrupt-from-foreign]]

**Shared mechanism:** `verify_deposit_message` + `verify_custody`
(`mod.rs:260-310`) — the S1–S4 checks that run on every custody call regardless
of attestation. **Dominance:** `stored-share-matches-committed-pubkey-share` (S4)
is the load-bearing safety barrier; `custody-rejects-invalid-deposit-data`
(S1–S3) is its weaker, largely unit-testable precondition.
`fork-version-source-consistency` modifies the S1 signing domain (so it also
links into Cluster 2's preimage correctness).

## Cluster 4 — Approval signature semantics

- [[approval-binding-and-replay-resistance]]
- [[approval-deterministic-and-idempotent]]

**Shared mechanism:** the `approve_custody` ECDSA output (`mod.rs:312-350`).
**Dominance/order:** `approval-deterministic-and-idempotent` is a *precondition*
for reasoning about `approval-binding-and-replay-resistance` — the "differs-only-in-
verify_session ⇒ identical bytes" (dark twin) and "differs-in-validator_index ⇒
different bytes" assertions only make sense if the signature is a deterministic
pure function of its inputs.

## Cluster 5 — Sign-exit capability

- [[sign-exit-requires-authorization]]
- [[sign-exit-index-binding]]

**Shared mechanism:** `sign_voluntary_exit_message` (`mod.rs:352-394`).
`sign-exit-requires-authorization` is the safety/authorization concern (ungated,
epoch-0, replayable); `sign-exit-index-binding` is the liveness concern (a share
stored at index i>0 can't be found when reef asks for index 0). Both feed the
master invariant in Cluster 1.

## Cluster 6 — Persistence integrity & crash recovery

- [[key-write-durable-or-rejected]] *(crash face)*
- [[torn-read-never-yields-wrong-key]] *(concurrency face)*
- [[no-orphaned-unacknowledged-secret]]
- [[missing-state-request-fails-clean]]

**Shared mechanism:** the `fs::write` key store with no fsync/atomic-rename/lock
(`key_management.rs:9-14`). **Dominance:** `key-write-durable-or-rejected` and
`torn-read-never-yields-wrong-key` are the two faces of non-atomic writes (crash
vs concurrent reader) — neither dominates; both share the "empty/truncated file
→ blsttc zero-key" escalation, so a fix (atomic write + integrity check) closes
both. `no-orphaned-unacknowledged-secret` stems from write-before-success
ordering and is the recovery/liveness counterpart.

## Cluster 7 — Availability & resource bounds

- [[dependency-hang-makes-progress]]
- [[upcheck-live-under-load]]
- [[bounded-resource-usage]]

**Shared mechanism:** blocking work (fs, ECIES, BLS) on the async workers with no
`spawn_blocking`, no request/dependency timeouts, and no resource limits.
`dependency-hang-makes-progress` (a hung dep pins a worker) and
`upcheck-live-under-load` (head-of-line starvation) are the same starvation
mechanism from two triggers; `bounded-resource-usage` is the slow-accumulation
variant.

## Cluster 8 — Input robustness & lifecycle

- [[malformed-input-never-panics]]
- [[startup-bind-failure-not-silent]]
- [[guardian-routes-time-independent]]

**Shared mechanism:** edge handling (panic on bad input, silent bind failure,
absence of time dependence). `malformed-input-never-panics` ↔
`missing-state-request-fails-clean` (Cluster 6) are siblings — both assert "fail
cleanly, never panic," from different triggers (bad input vs missing state).

## Cross-cluster connections worth testing together

- **The catastrophe chain (P1 + Cluster 6 + Cluster 5)** — now a first-class
  property: [[unattested-share-cannot-become-exit-capability]] (added in
  evaluation, gap G2). attestation OFF (P1) → `no-orphaned-unacknowledged-secret`
  persists an unattested share → `sign-exit-requires-authorization` mints an exit
  from it. The new property names this as one testable end-to-end timeline; the
  three component properties (`attestation-verified-before-approval`,
  `no-orphaned-unacknowledged-secret`, `sign-exit-requires-authorization`) are its
  unit-level decomposition (whether to keep both layers is curation Bias B1).
- **The provisioning-liveness chain (Cluster 2 + Cluster 4):**
  `custody-preimage-matches-onchain-verifier` drift only manifests as a
  protocol-wide stall *because* `approval-deterministic-and-idempotent` guarantees
  every honest guardian produces the same (rejected) bytes — i.e. the bug is
  correlated, not flaky.
- **The non-atomic-write chain (Cluster 6 ↔ Cluster 7):**
  `torn-read-never-yields-wrong-key` requires the concurrency that
  `upcheck-live-under-load`'s load scenario naturally produces; run them on the
  same workload.
- **The fork-domain link (Cluster 3 ↔ Cluster 2):**
  `fork-version-source-consistency` feeds the deposit domain that
  `custody-rejects-invalid-deposit-data` (S1) checks and that the preimage
  properties encode.
- **The ignored-clock link (Cluster 8 ↔ Cluster 1):**
  `guardian-routes-time-independent` localizes the *one* time field the system
  ignores — session `expiresAt` — which is exactly the gap in
  `session-verification-fails-closed`.

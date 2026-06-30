---
sut_path: /home/fawad/puffer/projects/validator-guardian
commit: f7dbd88d99de21a6d8caba5f7de8216624ab1098
updated: 2026-06-30
external_references:
  - path: /home/fawad/puffer/projects/reef
    why: consumer call patterns; the orphaned-attestation seam; deployed contract addresses.
  - path: /home/fawad/puffer/projects/coral
    why: confirms coral does not call the guardian.
  - path: /home/fawad/puffer/projects/puffer-contracts
    why: Foundry broadcast artifacts indicating which GuardianModule bytecode was deployed (W0 liveness).
---

# Property Evaluation — Synthesis

Four adversarial lenses (antithesis-fit, coverage-balance, implementability,
wildcard) evaluated the 27-property catalog as a portfolio. Per-lens evidence in
`evaluation/{lens}.md`. Findings categorized below as **Gap** (expand the
catalog), **Bias** (needs human judgment), or **Refinement** (apply directly).

## Overall verdict

The catalog is strong and accurate; no property was invalidated. The evaluation
produced **2 gaps**, **3 biases for human judgment**, and **~10 refinements**.
Two cross-lens themes dominate:

1. **The live vs. inert split is mis-drawn in places.** The catalog treats the
   attestation story as uniformly "inert under P1 (verify_session=false)." But the
   **keygen → `rotateGuardianKey`** evidence path is **verified on-chain today**
   (coverage CW-1, wildcard F-FIND-4), and `rotate-key-preimage-matches-onchain` /
   `cvm-stub-never-in-production` are gated by `CVM_AGENT_STUB`, **not**
   `verify_session` — so they are live, not inert. → Refinements R1, and Gap G1.
2. **Antithesis-fit skew.** Only ~6 of 27 properties genuinely require an
   Antithesis-only capability (concurrency + crash + liveness); ~13 are
   deterministic oracles / regression checks (antithesis-fit CW1). This is partly
   inherent to the SUT (strong crypto checks, weak fault/persistence handling),
   but it is a portfolio-orientation question for the human → Bias B1.

---

## GAPS (fill via targeted discovery)

### G1 — keygen → `rotateGuardianKey` on-chain evidence round-trip *(live today)*
**From:** coverage CW-1, wildcard F-FIND-4.
**Concern:** `KeyGenResponse { pk_hex, evidence }` is consumed by reef
(`rotate_guardian_key.rs:88-195`), repackaged into a `GuardianSessionProof`, and
verified on-chain by `GuardianModule.rotateGuardianKey()`. The catalog's
`rotate-key-preimage-matches-onchain` covers only the keccak `signedMessageHash`,
not the `owner_public_key`/`session_public_key` `PublicIdentity` round-trip
(contract requires `typeId==ES256K`, 65-byte key) nor the `pk_hex`
uncompressed→`ecies::parse` round-trip. This is the **keygen twin of the critical
W0 custody-preimage property — and it is on-chain-verified in production now**,
unlike the custody path.
**Action:** ADD property `keygen-evidence-accepted-onchain` (done — see catalog
Category B + evidence file).

### G2 — the catastrophe chain has no first-class catalog row
**From:** wildcard F-FIND-6 (+ relationships file already flags it as the highest-
value end-to-end timeline).
**Concern:** "attestation-off → orphaned/unattested share persisted → sign-exit
mints an exit from it" exists only as prose in `property-relationships.md`. The
single best end-to-end Antithesis scenario is the least likely to get built
because no property names it.
**Action:** ADD property `unattested-share-cannot-become-exit-capability` as an
explicit end-to-end timeline (done — see catalog Category E + evidence file).

### G3 (folded, not a new property) — base64/hex deserialization-failure boundary
**From:** coverage F-1. A wrong-encoding `guardian_enclave_public_key` fails in the
axum `Json` extractor (422/400) *before* the handler — distinct from the
post-deserialization F7/F8/F11 panics. **Action:** folded into
[[malformed-input-never-panics]] scope (extractor-level malformed input → clean
4xx, no panic) rather than a separate property.

---

## BIASES (human judgment required — see "Decisions for the user" at end)

### B1 — portfolio orientation: oracle/regression vs fault-sensitive
**From:** antithesis-fit CW1.
**Evidence:** ~13 properties are self-described as "not fault injection" /
"unit-testable" / "coverage+regression" (the three preimage properties,
`custody-rejects-invalid-deposit-data`, `session-public-key-parse`,
`approval-binding`, `pubkey-representation`, `guardian-routes-time-independent`,
`cvm-stub`, `no-request-authentication`). Only the concurrency + crash + liveness
cluster (~6: torn-read, key-write-durable, no-orphaned, upcheck-live,
dependency-hang, missing-state) and the index-binding bug need Antithesis's
differentiated search.
**The judgment:** keep the broad catalog (the oracle properties are cheap,
high-severity regression guards and Antithesis's diverse input generation does add
value over fixed unit tests), OR trim to the fault-sensitive core for the
Antithesis harness and run the oracle properties as ordinary CI tests. Note the
SUT genuinely *is* oracle-heavy (it's a near-stateless signer with strong crypto
checks and weak fault/persistence handling), so some skew is inherent.

### B2 — trust-boundary / severity framing is layer-specific and partly overstated
**From:** wildcard F-FIND-3 (+ F-FIND-2).
**Evidence:** the SUT analysis says "no timeout anywhere → stalls indefinitely"
and frames the unauthenticated guardian as the trust boundary (P2, OQ4). But the
*real* boundary is two layers up — **puffer-ingestor + Lambda/cron** (never named
in the analysis); reef's catastrophe-chain webhooks **are** API-key-authenticated;
and ingestor→reef has a **30s timeout**. So `dependency-hang-makes-progress`'s
"indefinite stall" is layer-local to guardian↔reef, and OQ4 ("untrusted clients?")
is likely aimed at the wrong layer. Also F-FIND-2: the guardian's verify path is
**doc-disowned** (`migration-guide-tdx.md:244` says the *caller* verifies; reef
has zero SessionRegistry code) — it is orphaned dead code, not a "future
guardrail."
**The judgment:** how much should the catalog/severities lean on the guardian's
own lack of auth/timeouts, given the upstream auth + timeout? Does the test target
need to include the ingestor layer, or is guardian-in-isolation correct?

### B3 — scope: is "guardian binary only" the right test target?
**From:** wildcard framing-question + F-FIND-1/F-FIND-5.
**Evidence:** the single most severe finding (W0 preimage drift) lives in
**puffer-contracts**, not the guardian; its resolution and the two preimage
properties only mean something against a deployed `GuardianModule`. The
guardian-only frame pushes the highest-severity, most-actionable bug to an "open
question."
**The judgment:** should the first harness be **guardian + anvil + real
GuardianModule** as one system (making W0 a first-class testable assertion), or
guardian-in-isolation with a Rust reference encoder standing in for the contract
(cheaper; catches W0 deterministically but doesn't prove the deployed bytecode)?

---

## REFINEMENTS (applied directly to catalog/topology/relationships)

- **R1 — re-tag the CVM_AGENT_STUB-gated properties as LIVE.**
  `rotate-key-preimage-matches-onchain` and `cvm-stub-never-in-production` are
  gated by `CVM_AGENT_STUB`, **not** `verify_session`, and run on the live
  keygen→`rotateGuardianKey` path. Removed the "inert today / on verify path"
  qualifier from both. *(antithesis-fit, wildcard F-FIND-4, coverage CW-1)*
- **R2 — custody-preimage primary form = no-chain reference encoder.** The W0
  property's primary, cheapest check is a Rust/`alloy sol!` reference encoder
  differentially compared to `approve_custody` (catches drift deterministically,
  no chain); a minimal **GuardianModule-only** anvil deploy is the secondary form;
  the full SessionRegistry tree is a separate sized work item. Applied to the
  property and to `deployment-topology.md`. *(implementability CW-2)*
- **R3 — the two preimage properties are an unsatisfiable conjunction (XOR), not
  independent siblings.** The W0 dual-bind means no single deployed contract
  satisfies both the 7-field custody preimage and the `ROTATE_GUARDIAN_KEY` rotate
  format. Noted in the catalog and `property-relationships.md` Cluster 2.
  *(wildcard F-FIND-5)*
- **R4 — elevate `approval-deterministic-and-idempotent` Medium → High.** It is the
  load-bearing precondition for the assertion *semantics* of two Critical
  properties (a P5-style precondition). *(antithesis-fit, wildcard F-FIND-7)*
- **R5 — demote/annotate the oracle/documenting properties.** Marked
  `custody-rejects-invalid-deposit-data`, `session-public-key-parse-no-wrong-identity`,
  `guardian-routes-time-independent`, `cvm-stub-never-in-production` (config check),
  and `no-request-authentication` as **regression/documenting — low Antithesis
  priority, runnable as ordinary tests**. `no-request-authentication` explicitly
  noted as a duplicate of precondition P2, not an independent property (kept as a
  documenting row pending Bias B1's resolution). *(antithesis-fit, wildcard
  F-FIND-9)*
- **R6 — reinforce node-termination + exec-form ENTRYPOINT + two volume configs.**
  `deployment-topology.md` already flags node-termination; added the exec-form
  ENTRYPOINT requirement (binary isn't PID 1 today) and the durable-vs-reset
  `/data` two-config need. *(implementability CW-4)*
- **R7 — verify-path session-key precondition.** The in-repo sim CVM agent
  generates its session key with `random(OsRng)` per registration — NOT a fixed
  harness keypair. Added the seed-vs-dynamic-register decision to
  `deployment-topology.md`. *(implementability CW-3)*
- **R8 — mock CVM agent can't catch real-agent drift.** The mock signs keccak256
  by construction, so `rotate-key-preimage-matches-onchain` /
  `attestation-payload-reconstruction-matches` cannot catch a *real* DCAP-agent
  byte drift via the mock — the differential value is the Rust↔contract reference
  encoder, not the mock. Noted in topology + both properties. *(wildcard F-FIND-10)*
- **R9 — `malformed-input-never-panics` high-fidelity form needs `tower-http`.** The
  clean `CatchPanicLayer`→`assert_unreachable!` form requires adding `tower-http`
  (no tower dep today) and the panic sites are inside `async` fns (awkward
  `catch_unwind`); the fallback is a lower-fidelity workload-side connection-reset
  proxy. Noted in the property. *(implementability CW-1)*
- **R10 — `bounded-resource-usage` has a host-metric half.** FD/RSS observation is
  a host/container metric, not an SDK assertion; noted in the property.
  *(implementability CW-7)*
- **R11 — W0 deployment is mechanically verifiable, not pure human-input.** On-disk
  Foundry broadcast (`puffer-contracts/.../broadcast/DeployEverything.s.sol/560048/
  run-latest.json`, Hoodi, 2026-03-10) deployed a GuardianModule from the 5-field
  `feat/tdx` checkout; reef pins the addresses (Hoodi `0x7c35…19De`, mainnet
  `0x628b…CcF2`). Catalog Q1 updated: the deployed field-count is confirmable with
  a one-line `cast call` / `eth_getCode` (proxy-upgrade caveat U1), not a
  product-owner question. *(wildcard F-FIND-1)*

## Re-evaluation note

The two gap-fill properties (G1, G2) are in existing categories (B and E) and
reuse confirmed evidence; per the skill's threshold (a 1-3 property gap in an
existing category does not warrant a second full evaluation pass), no second
evaluation round was run. The refinements are local and do not alter the
portfolio's shape.

## Open uncertainties carried forward (from the lenses)

- U1: is the deployed Hoodi/mainnet GuardianModule a proxy that's been upgraded
  since the 2026-03-10 broadcast? (needs `eth_getCode`/`cast`).
- U2: puffer-ingestor inbound authentication.
- U3: do beacon-chain exits require current-epoch (vs the hardcoded epoch-0)? —
  promotes `guardian-routes-time-independent` from canary to High.
- U4: production M-of-N.
- U5: is a zero-key (`Fr::zero()`) share exploitable at the on-chain aggregator
  (a publicly-computable share — forgery seam)? (wildcard F-FIND-11)
- U6: reef-guardian centralized vs edge topology (who can reach the guardian port).

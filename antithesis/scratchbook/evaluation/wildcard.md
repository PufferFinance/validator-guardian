---
sut_path: /home/fawad/puffer/projects/validator-guardian
commit: f7dbd88d99de21a6d8caba5f7de8216624ab1098
updated: 2026-06-30
external_references:
  - path: /home/fawad/puffer/projects/reef
    why: the real trust boundary / consumer
  - path: /home/fawad/puffer/projects/coral
    why: confirms coral does not call the guardian
  - path: /home/fawad/puffer/projects/puffer-contracts
    why: W0 verifier; Hoodi DeployEverything broadcast pins the deployed 5-field bytecode
  - path: /home/fawad/puffer/projects/puffer-ingestor
    why: the actual orchestrator ABOVE reef — the layer the catalog never models
---

# Wildcard Evaluation — `guardian` binary property catalog

> Lens charter: find what the other three lenses (Antithesis-Fit, Coverage-Balance,
> Implementability) structurally cannot, because all three accept the SUT analysis
> and the "guardian binary only" scope. My job starts where theirs ends: question
> the framing, find missing angles, cross-cut the lenses, report what is odd.

This catalog is **unusually good** — the investigation pass is honest, the
preconditions P1–P4 are exactly the right gating facts, and the per-property
evidence files are rigorous. So the wildcard value here is not "the catalog is
wrong"; it is "the catalog's *frame* leaves several high-leverage things either
mis-bucketed, double-counted, or — for the single most important question —
declared 'human-input unknown' when it is actually answerable." Everything below
is verified firsthand against the SUT and the four external repos.

---

## A. Questioning the framing

### W-F1. "Guardian binary only" hides that verification is orphaned across the guardian↔reef seam — not merely "off"

The catalog's P1 says attestation is "OFF in production today" because reef sends
`verify_session=false`. True, but the frame stops one repo too early. The TDX
migration guide *in this very repo* (`docs/migration-guide-tdx.md:244`, currently
being edited on this branch) states the intended design:

> "The guardian's `verify_session_evidence()` currently performs structural
> validation only … Full on-chain verification via
> `SessionRegistry.verifySessionSignature()` is **deferred to the caller**."

So the documented contract is: **the caller (reef) does the on-chain session
verification.** I grepped reef for any SessionRegistry / `verifySessionSignature`
usage: there is **none** (only `new_registration.rs:266 verify_session: false`).
So the picture is not "attestation is a feature toggled off pending P1" — it is
"attestation verification belongs to no component at all": the guardian's path is
gated off, the migration guide pushes the responsibility to the caller, and the
caller never implements it. This matters for the catalog because it reframes
**which component** Category A's properties should target. The "fail-closed
verify path" (`session-verification-fails-closed`, `attestation-payload-
reconstruction-matches`, the verify-half of `custody-preimage`) are framed as
"inert until reef flips P1 on." But there is no evidence reef is being built to
flip it on — there is positive evidence (the migration guide) that the design
intends the *caller* to verify, which means flipping `verify_session=true` may
never be the plan, and the guardian's whole SessionRegistry `eth_call` machinery
is **half-built dead code that the docs already disown.** That is a stronger,
more actionable statement than "inert pending a product decision," and it shifts
the honest framing of ~4 properties from "aspirational guardrail for the future"
to "guarding a path the architecture has tacitly abandoned." The catalog's OQ3
("is verify_session intended to be flipped on?") should be re-pointed: the
question is not the guardian flag, it is **"who owns attestation verification,
and why does neither the guardian nor reef do it?"**

### W-F2. The real trust boundary is two layers up (puffer-ingestor + Lambda), and reef *is* partly authenticated — the catalog's "unauthenticated orchestrator" is imprecise in both directions

The catalog (P2, `no-request-authentication`, SUT §5) frames the trust boundary
as "the guardian trusts an unauthenticated reef/BFF orchestrator." Cross-repo
verification refines this materially:

- **reef is NOT unauthenticated on the paths that drive the catastrophe chain.**
  Both privileged webhooks gate on an API key:
  `provision_or_skip/handler.rs:36 api_key_check(...)` and
  `eject_validator.rs:40 api_key_check(...)`. So the entry to "force-eject a live
  validator" requires reef's API key, not raw network reach.
- **The real orchestrator is puffer-ingestor, a layer the catalog never names.**
  puffer-ingestor calls reef with the API key
  (`puffer-ingestor/src/services/reef/mod.rs:16` — and notably a **30s timeout**,
  `eject_validator.rs`/`provision_or_skip.rs`), and ingestor itself is triggered
  by AWS Lambda / scheduled bash scripts whose *inbound* auth is undocumented.
  The ejection decision is data-driven from ingestor's DB
  (`validator_eth_balance_check.rs` — eject validators low on ETH).
- **reef→guardian is the bare, no-timeout, no-auth hop** (confirmed:
  `ClientBuilder` → `reqwest::Client::new()`, no `.timeout()`, no auth header on
  `validate-custody`/`sign-exit`/`keygen`).

Net: the trust chain is **Lambda/cron → (auth?) → puffer-ingestor → (API key, 30s
timeout) → reef → (bare, no-timeout, no-auth) → guardian.** This changes two
things the catalog gets slightly wrong:

1. The "no timeout anywhere" claim that strengthens the liveness cluster
   (`dependency-hang-makes-progress` OQ "RESOLVED: NO") is **layer-specific**:
   reef→guardian has no timeout, but **ingestor→reef has a 30s timeout.** So a
   hung guardian during keygen does NOT stall the whole pipeline indefinitely —
   it stalls reef's request, and ingestor gives up after 30s (then presumably
   retries on the next schedule, which feeds the orphan-accumulation story).
   The catalog overstates "unrecoverable without operator action."
2. Severity of the whole "attacker-controlled field" cluster (OQ4) should be
   evaluated against **puffer-ingestor's inbound auth**, not the guardian's port.
   The guardian's no-auth is genuinely defense-in-depth *behind two authenticated
   hops*; the actual soft target is ingestor's inbound endpoints. The catalog's
   single most-cited gating question (OQ4) is pointed at the wrong layer.

**This is the clearest "guardian binary only" scope miss.** The catalog correctly
reframed the boundary onto "reef/BFF + contracts," but did not go the one step
further to discover that reef is authenticated and ingestor is the real
soft-edge. A reader optimizing test budget on "guardian endpoint reachable by
anyone" is mis-calibrated.

### W-F3. The test target SHOULD be guardian+anvil+contracts+a reef-shaped client — and the topology doc already says so, but with a latent self-defeating choice

The deployment-topology doc already proposes the right system-level harness
(workload + guardian + anvil-with-real-contracts). Good. But there is a buried
contradiction the lenses miss: the topology says deploy `SessionRegistry +
GuardianModule from puffer-contracts` (`deployment-topology.md:80`) and the repo
is checked out on `feat/tdx` — the **5-field** branch. If the harness deploys the
`feat/tdx` GuardianModule, then `custody-preimage-matches-onchain-verifier`'s
on-chain round-trip will **deterministically reject every approval** — which is
"correct detection of W0," but only if the workload author realizes that the
failing assertion is the *finding*, not a harness bug. If they instead deploy the
`fix/increase-guardian-signatures-security` contract to "make the test pass,"
they will have silently tested against a contract that **does not exist on any
deployed chain** and masked W0 entirely. The catalog never tells the harness
author which branch to deploy or *why the choice is itself the experiment.* This
should be called out explicitly in the topology (see Suggested action under F-FIND-1).

---

## B. Missing angles

### W-M1. W0 is presented as a "human-input unknown" but is concretely resolvable — and there is positive evidence the 5-field (rejecting) verifier was actually deployed

The catalog repeatedly calls "which bytecode is deployed?" the single most
important question and marks it **needs human input** (catalog OQ1, repeated in 4
files). My cross-repo dig found this is **answerable from artifacts already on
disk**, and the answer leans toward "the rejecting one is deployed":

- `puffer-contracts/mainnet-contracts/broadcast/DeployEverything.s.sol/560048/run-latest.json`
  is a Foundry broadcast that **deployed a `GuardianModule` to chain 560048
  (Hoodi testnet) on 2026-03-10**, from this exact `feat/tdx` checkout — whose
  `LibGuardianMessages._getBeaconDepositMessageToBeSigned` is the **5-field**
  preimage (verified firsthand: `LibGuardianMessages.sol:32` =
  `keccak256(abi.encode(pufferModuleIndex, pubKey, withdrawalCredentials,
  signature, depositDataRoot))`).
- reef pins the **deployed addresses**: Hoodi GuardianModule
  `0x7c3593C2c80Fe45bcDa0A0D5052e8d87d9EE19De`, mainnet
  `0x628b183F248a142A598AA2dcCCD6f7E480a7CcF2`
  (`reef-guardian/src/constants/{staging/hoodi,production/mainnet}/protocol.json:4`).
- reef-lib pins the **guardian** to branch `feat/tdx-improve-signature` (the
  7-field signer): `reef/reef-lib/Cargo.toml:36`.

So on Hoodi the deployed verifier appears to be 5-field while the consumed
guardian is 7-field → **W0 is not hypothetical, it is evidenced as live on at
least one chain.** The truly remaining unknown is only whether the GuardianModule
is behind an upgradeable proxy that was later pointed at 7-field bytecode — which
is resolvable by a single `eth_getCode`/`cast call` against the pinned address, or
by checking the Deployments-and-ACL repo. The catalog should downgrade OQ1 from
"needs human input" to "needs a one-line on-chain query (here is the address)."
This is the highest-leverage correction in this report: the catalog's #1 question
has a cheap mechanical answer it never attempts.

### W-M2. The catalog under-weights that ONE side of the attestation story is LIVE, while treating the whole of A/B as P1-inert

The catalog's priority annotations repeatedly tag Category-A/B properties "inert
if attestation off" — and apply that tag too broadly. There are **two distinct
attestation paths**, gated by **two different flags**, and only one is inert:

- **Custody verify path** — gated by request `verify_session` (reef sends
  `false`) → genuinely inert. (`session-verification-fails-closed`,
  `attestation-payload-reconstruction-matches`, verify-half of
  `attestation-verified-before-approval`.)
- **Keygen rotate path** — gated only by env `CVM_AGENT_STUB`, **NOT** by
  `verify_session`. reef **actively exercises it in production**:
  `reef-guardian/src/handlers/api/guardian/rotate_guardian_key.rs:88` calls
  keygen, `:191` submits `GuardianModule.rotateGuardianKey(...)` on-chain with the
  CVM session evidence. So `rotate-key-preimage-matches-onchain` and
  `cvm-stub-never-in-production` test **LIVE production code today**, not "inert
  pending P1."

Yet `rotate-key-preimage-matches-onchain` carries the priority note "*but on the
keygen-attestation path; inert if attestation off*" (catalog line 190) — which is
**wrong**: it is not gated by `verify_session` at all and runs every time a
guardian rotates its key. This mislabeling could cause the synthesis/workload
step to deprioritize a property that actually guards a live on-chain transaction.
The rotate path is also where the W0 *dual-bind* bites in the live direction (see
W-X1). The catalog should split "attestation" into "custody-verify (inert)" vs
"keygen-rotate (live)" and re-tag accordingly.

### W-M3. A failure scenario nobody modeled: the W0 dual-bind makes provisioning AND key-rotation mutually unsatisfiable on any single deployed contract — this is a fork-coupled deadlock, not two independent liveness bugs

`custody-preimage-matches-onchain-verifier` and `rotate-key-preimage-matches-
onchain` are cataloged in Cluster 2 with the explicit note **"No dominance —
each targets a different external boundary."** That is the miss. They are not
independent: the evidence files themselves establish (and I re-confirmed) that
**no single puffer-contracts branch satisfies both** —

- `feat/tdx`: rotate-key MATCHES (7-field rotate verifier present), custody
  DRIFTS (5-field custody verifier).
- `fix/increase-guardian-signatures-security`: custody MATCHES (7-field), rotate
  has **no `ROTATE_GUARDIAN_KEY` path at all**.

So whichever contract is deployed, **exactly one of {provision a new validator,
rotate a guardian key} is on-chain-broken.** This is a single
catastrophe-class property — "the guardian's two on-chain preimages cannot both
verify against any one deployed contract" — that the catalog has split into two
"no-dominance siblings," thereby hiding the most interesting fact about them:
their conjunction is unsatisfiable. The right Antithesis encoding is one
differential test that deploys a *chosen* contract branch and asserts the
**XOR**: `Sometimes("custody verifies on-chain")` and `Sometimes("rotate verifies
on-chain")` must **not both be Reachable against the same deployed bytecode.** No
single lens constructs this because Coverage-Balance sees two well-formed
properties, Antithesis-Fit sees two reasonable differential tests, and
Implementability sees two anvil deployments — only stitching them reveals the
contradiction.

### W-M4. The catalog tests "torn-read zero key" and "crash → zero key" as separate properties but never models the on-chain consequence of a zero-key signature

`torn-read-never-yields-wrong-key` and `key-write-durable-or-rejected` both
escalate to "blsttc deserializes empty/truncated → `Fr::zero()` → HTTP 200 with a
zero-key signature share." Good catch. But the analysis stops at the guardian's
HTTP boundary ("corrupts threshold exit signing"). Nobody traced what a zero-key
*share* does at the on-chain aggregator. A zero secret key produces a
**deterministic, publicly-computable** signature share (anyone can sign with
`Fr::zero()`), and its `public_key_share` is the identity element. If the
aggregator or the BLS threshold scheme treats an identity-element share specially
(or if multiple guardians independently emit the *same* zero share under a
correlated truncation, given P3 same-binary), the threshold math may behave in a
way that is either harmless (rejected) or a forgery seam (a predictable share an
attacker can supply without holding any key). This is exactly the kind of
cross-boundary consequence the system-level harness (guardian+contracts) could
witness but no property currently asks about. At minimum it is an open question
worth flagging; the catalog currently asserts the harm ("corrupts threshold
signing") without modeling whether it's a liveness annoyance or a safety hole.

---

## C. Cross-cutting the lenses

### W-X1. The "catastrophe chain" is documented as prose in property-relationships but is NOT represented as a single testable end-to-end timeline property

`property-relationships.md` describes "the catastrophe chain (P1 + Cluster 6 +
Cluster 5)" — attestation off → orphaned share persisted → sign-exit mints an
exit from it — and calls it "the highest-value end-to-end timeline." And
`attestation-verified-before-approval.md` (the W7-6 master section) states the
master lifecycle invariant beautifully. **But there is no catalog entry whose
*property statement* is the end-to-end timeline.** The 27 properties are all
local (single endpoint, single mechanism). The chain exists only as connective
prose between them. For Antithesis specifically this is a real gap: Antithesis's
differentiating strength is finding the *interleaved multi-step* path
(crash-after-write, then cross-endpoint sign-exit on the orphan), and that is
precisely the thing represented nowhere as an assertable property. The
master `Always(exit_signed ⇒ share_was_verified)` is mentioned inside
`attestation-verified-before-approval` as "w7-6" but it is buried as one bullet in
a merged section, not elevated to a first-class property with its own slug,
priority, and instrumentation plan. Result: the single highest-value Antithesis
scenario is the least likely to actually get built, because no catalog row owns
it. This is a structural cross-cut: Antithesis-Fit rates each local property
fine; Coverage-Balance counts the components as covered; only tracing the
*lifetime of one share* (which the wildcard remit demands) shows the end-to-end
property is missing from the enumerated set.

### W-X2. `approval-deterministic-and-idempotent` is the hidden keystone for THREE other properties — it is rated Medium but is a precondition that, if false, invalidates their assertions

The catalog rates `approval-deterministic-and-idempotent` Medium and treats it as
a standalone idempotency check. But determinism is the **enabling precondition**
for the assertion *logic* of at least three other properties, and the
relationships doc only notes one of the three:

- `approval-binding-and-replay-resistance`: the "dark twin" (`Always` identical
  when only `verify_session` toggles) and "differs when `validator_index`
  differs" assertions are **meaningless unless** the signature is a deterministic
  pure function. (relationships doc notes this one.)
- `custody-preimage-matches-onchain-verifier` (the coordination half):
  "every honest guardian produces the byte-identical (rejected) signature" — the
  *correlated* nature of W0 that makes it catastrophic rather than flaky —
  **depends on** determinism. (relationships doc notes this as the
  "provisioning-liveness chain.")
- `sign-exit-requires-authorization`: "infinitely replayable byte-identical exit
  share" — the thing that makes a captured share a *permanent* capability —
  **depends on** BLS-signing determinism.

So determinism is load-bearing for the entire severity argument of the two
Critical properties and one High property, yet it is rated Medium and bucketed in
its own Cluster 4. Under a cross-lens view it is closer to a **catalog-wide
precondition (a P5)** than a standalone property. If Antithesis ever observed
non-determinism, it would not just fail one Medium property — it would silently
invalidate the assertion *semantics* of the two Critical ones. Recommend
promoting it to a precondition-grade anchor (and instrumenting it first, as the
replay anchor the catalog already half-acknowledges).

### W-X3. `guardian-routes-time-independent` (Low) becomes the canary for a real correctness bug under a topology Implementability deems feasible

The catalog rates `guardian-routes-time-independent` Low — "negative-space
invariant / regression tripwire." But cross-cutting with the *hardcoded epoch-0*
finding (sign-exit signs `VoluntaryExit{epoch: 0}`) flips its value. If the beacon
chain requires the exit epoch to be the **current** epoch (an open question the
catalog flags but does not resolve), then the guardian's exits are **invalid on
the consensus layer** — and `guardian-routes-time-independent`'s "looks
deterministic under clock jitter" assertion would *pass* while masking that the
output is semantically wrong (epoch frozen at 0 regardless of real time). Under
the system-level harness (which Implementability says is feasible), a workload
could submit a guardian-produced exit to a beacon-shaped verifier and discover
epoch-0 is rejected — turning a Low "tripwire" into the witness for a High
correctness bug. The catalog notes the epoch-0 question in passing but never
connects it to making this Low property high-value; that connection only appears
when you cross the time-independence lens with the sign-exit-correctness lens.

---

## D. What is odd (fits no category)

### W-O1. 27 is slightly too many for one search budget — and the surplus is concentrated in low-value documenting/latent properties, not the priority area

27 properties dilute the Antithesis search budget. The honest investigation pass
already downgraded several to Low/latent; a few are arguably not "properties":

- `no-request-authentication` is explicitly **"a documented bug with no
  meaningful assertion"** — the catalog itself says the `Reachable` "would always
  fire" and is "arguably low-value to instrument" (`no-request-authentication.md`
  Instrumentation note). It is a *precondition* (it duplicates P2) wearing a
  property costume. Recommend demoting it to a precondition cross-reference, not a
  catalog row — it currently inflates the count and will burn instrumentation
  attention on an assertion that conveys no search signal.
- `pubkey-representation-consistent` (latent, not live — reef round-trips
  uncompressed), `session-public-key-parse-no-wrong-identity` (downgraded to
  liveness-only, JSON branch dead for SUT payloads), and
  `guardian-routes-time-independent` (Low tripwire) are all honestly-flagged as
  low-yield. Three Low + one duplicate-of-precondition = the catalog could shed
  ~3–4 rows with near-zero coverage loss, refocusing budget on the priority area.

This is not "the catalog is padded" — every property is justified — it is "the
*portfolio* leans 1–2 properties too far toward documenting completeness over
search yield," and the trimmable ones cluster *away* from the stated priority
(attestation/signature), which is the opposite of what a priority-weighted
portfolio wants.

### W-O2. Two property *pairs* are close to "secretly the same" mechanism — worth a conscious merge-or-keep decision

- `key-write-durable-or-rejected` (crash face) and
  `torn-read-never-yields-wrong-key` (concurrency face) share the **identical**
  root cause (`fs::write` O_TRUNC, no fsync/atomic/lock → blsttc zero-key) and the
  **identical** dangerous outcome (zero-key 200). The relationships doc admits "a
  fix … closes both." They differ only in *trigger* (crash vs concurrent reader).
  Antithesis will likely surface the same `Fr::zero()` assertion from both. Keep
  both only if node-termination (crash) and thread-pause (race) are *both* enabled
  faults; if only one fault class is enabled, the other property passes vacuously
  and the pair collapses to one. Flag this dependency explicitly.
- `attestation-verified-before-approval` (the W7-6 master) and
  `sign-exit-requires-authorization` are two views of the *same* unenforced
  invariant ("no signing capability without prior verification"). The master
  property's own text says sign-exit-no-auth *is* step 3 of its chain. They are
  not duplicates, but the boundary between "the master lifecycle invariant" and
  "the sign-exit local property" is blurry enough that a reader may instrument the
  same `Reachable("exit produced for unattested share")` twice under two slugs.

### W-O3. The harness's mock CVM agent quietly defines away `rotate-key-preimage-matches-onchain`'s entire reason to exist

`rotate-key-preimage-matches-onchain` exists to catch drift between the guardian's
keccak preimage and **the real DCAP CVM agent's** signing behavior (the property
file's own "version-compatibility hazard": the `automata-cvm-agent` dep is pinned
to an unpinned `branch = "dev"`). But the topology's mock CVM agent "signs
`keccak256(data)` with a fixed session keypair" (`deployment-topology.md:62`) —
i.e. the mock is built to agree with the guardian *by construction.* So the
harness can never observe the one drift this property was created to catch (the
real agent changing to EIP-191 or sha256). The property's *only* testable residual
in this harness is the contract-side preimage match (rotate verifier on
`feat/tdx`), which is already covered structurally. This is an odd
self-cancellation between the topology lens and the property's intent that no
single lens flags: Implementability says "mock is feasible," the property says
"guard against agent drift," and together they produce a test that cannot fail for
the stated reason. Worth noting so the team does not believe they are testing CVM
agent compatibility when they are testing their own mock.

---

## Structured summary

### Findings

**F-FIND-1 — [Property: custody-preimage-matches-onchain-verifier; rotate-key-preimage-matches-onchain; catalog OQ1]**
- *Concern:* The catalog's #1 question ("which bytecode is deployed?") is marked
  "needs human input," but it is mechanically resolvable, and on-disk artifacts
  already point to the rejecting (5-field) verifier being live on Hoodi. W0 is
  evidenced, not hypothetical.
- *Scope:* property-pair + catalog-wide framing.
- *Evidence:* `puffer-contracts/.../broadcast/DeployEverything.s.sol/560048/run-latest.json`
  (GuardianModule deployed to Hoodi 2026-03-10 from the 5-field `feat/tdx`
  checkout; `LibGuardianMessages.sol:32` = 5 fields). Deployed addresses pinned in
  `reef-guardian/src/constants/{staging/hoodi,production/mainnet}/protocol.json:4`
  (Hoodi `0x7c35…19De`, mainnet `0x628b…CcF2`). `reef/reef-lib/Cargo.toml:36` pins
  the 7-field guardian. Guardian 7-field preimage at `guardian/mod.rs:322-336`.
- *Suggested action:* Replace OQ1 "needs human input" with "run `cast code`/`cast
  call` against the two pinned addresses (or check Deployments-and-ACL)"; record
  the likely answer (5-field → custody broken). In the topology, state explicitly
  that the harness must deploy the **deployed** branch and that a failing on-chain
  custody round-trip **is the W0 finding**, not a harness bug.

**F-FIND-2 — [Catalog-wide framing; P1; Category A]**
- *Concern:* Attestation verification is not merely "off pending P1" — it is
  orphaned across the guardian↔reef seam. The migration guide says the *caller*
  verifies; the caller (reef) has no SessionRegistry code. So the guardian's
  verify path is doc-disowned dead code, not a future-on guardrail.
- *Scope:* catalog-wide (re-frames ~4 Category-A/B verify-path properties).
- *Evidence:* `docs/migration-guide-tdx.md:244` ("deferred to the caller");
  reef grep for `verifySessionSignature|SessionRegistry` = 0 hits; only
  `new_registration.rs:266 verify_session:false`.
- *Suggested action:* Re-point OQ3 from "is verify_session flipped on?" to "who
  owns attestation verification, given neither guardian nor reef performs it?"
  Mark the custody-verify path as "doc-disowned / likely abandoned," distinct from
  the live keygen-rotate path.

**F-FIND-3 — [Catalog-wide; P2; no-request-authentication; OQ4; liveness cluster]**
- *Concern:* The trust boundary is two layers higher than modeled (puffer-ingestor
  + Lambda/cron). reef *is* API-key authenticated on the catastrophe-chain
  webhooks; reef→guardian is the bare hop; ingestor→reef has a 30s timeout. OQ4
  is pointed at the guardian port when the soft target is ingestor's inbound.
- *Scope:* catalog-wide (recalibrates severity of the entire attacker-controlled-
  field cluster and the "no timeout anywhere" liveness claim).
- *Evidence:* `provision_or_skip/handler.rs:36` & `eject_validator.rs:40`
  (`api_key_check`); `puffer-ingestor/src/services/reef/mod.rs:16` (30s timeout);
  reef `ClientBuilder` → `reqwest::Client::new()` no timeout/auth; ingestor
  triggered by Lambda/cron per puffer-internal-docs.
- *Suggested action:* Add puffer-ingestor as a fifth external reference; re-point
  OQ4 at ingestor inbound auth; soften the "stalls indefinitely / unrecoverable"
  wording in `dependency-hang-makes-progress` to "stalls reef's request; ingestor
  times out at 30s and retries (feeding orphan accumulation)."

**F-FIND-4 — [Property: rotate-key-preimage-matches-onchain; cvm-stub-never-in-production]**
- *Concern:* Mis-tagged as "inert if attestation off." They are gated by
  `CVM_AGENT_STUB`, not `verify_session`, and run on the **live** keygen→
  rotateGuardianKey production path. Risk: deprioritized as inert when they guard
  a live on-chain tx.
- *Scope:* property pair.
- *Evidence:* `reef-guardian/.../rotate_guardian_key.rs:88` (calls keygen), `:191`
  (`rotate_guardian_key` on-chain with CVM evidence); guardian gating only at
  `remote_attestation.rs:31` (`CVM_AGENT_STUB`), independent of `verify_session`.
- *Suggested action:* Split "attestation" into custody-verify (inert) vs
  keygen-rotate (live); remove the "inert if attestation off" note from
  rotate-key (catalog line 190).

**F-FIND-5 — [Property pair: custody-preimage + rotate-key; Cluster 2 "no dominance"]**
- *Concern:* The two preimage properties are not independent — no single deployed
  contract satisfies both (the W0 dual-bind). Their conjunction is unsatisfiable;
  the catalog hides this by bucketing them as "no-dominance siblings."
- *Scope:* property pair → one cross-process XOR property.
- *Evidence:* `feat/tdx` has 7-field rotate + 5-field custody; `fix/…` has 7-field
  custody + no `ROTATE_GUARDIAN_KEY` (confirmed in both evidence files and via
  grep across branches).
- *Suggested action:* Add a single differential property asserting the XOR
  against one deployed bytecode: not both `Sometimes("custody verifies")` and
  `Sometimes("rotate verifies")` may be Reachable on the same contract.

**F-FIND-6 — [Catalog-wide; property-relationships "catastrophe chain"]**
- *Concern:* The highest-value end-to-end timeline (attestation-off → orphaned
  share → sign-exit) exists only as prose + a buried "w7-6" bullet, with no
  first-class catalog row, slug, priority, or instrumentation plan. The single
  best Antithesis scenario is the least likely to get built.
- *Scope:* catalog-wide (missing property).
- *Evidence:* `property-relationships.md:129-134`; `attestation-verified-before-
  approval.md` W7-6 merged section (master invariant present but not elevated).
- *Suggested action:* Promote the lifecycle invariant to its own property
  (`exit-capability-implies-verified-custody`) with the cross-endpoint workload
  scenario (crash after `mod.rs:85`, then sign-exit on the orphan) as its primary
  Antithesis angle.

**F-FIND-7 — [Property: approval-deterministic-and-idempotent]**
- *Concern:* Rated Medium, but it is the load-bearing precondition for the
  assertion *semantics* of two Critical properties (custody-preimage correlated
  rejection; approval dark-twin) and one High (sign-exit infinite replay). It is
  closer to a precondition (P5) than a standalone Medium property.
- *Scope:* cross-cluster precondition.
- *Evidence:* relationships doc "provisioning-liveness chain" + the dark-twin
  logic in `approval-binding-and-replay-resistance`; BLS/ECDSA determinism in
  `sign-exit-requires-authorization` investigation log.
- *Suggested action:* Re-frame as a catalog precondition / instrument first as the
  replay anchor; note that its failure invalidates the Critical properties' logic.

**F-FIND-8 — [Property: guardian-routes-time-independent + sign-exit epoch-0]**
- *Concern:* Rated Low as a tripwire, but if the beacon chain requires
  current-epoch exits, this property's "deterministic under clock jitter" passes
  while masking that epoch-0 exits are consensus-invalid — a High correctness bug
  the system harness could witness.
- *Scope:* cross-property (time-independence × sign-exit correctness).
- *Evidence:* `mod.rs:364 sign_vem(sk, 0, …)`; catalog flags the epoch-0 question
  but never links it to this property's value.
- *Suggested action:* Note the dependency; resolve "is current-epoch required?"
  with the consensus owner; if yes, this Low becomes the canary for a High bug.

**F-FIND-9 — [Catalog portfolio size / no-request-authentication + Low cluster]**
- *Concern:* 27 dilutes search budget; `no-request-authentication` is a
  precondition (duplicates P2) with a self-admittedly always-firing assertion, not
  a property; 3 honestly-Low/latent properties cluster away from the priority area.
- *Scope:* catalog-wide portfolio.
- *Evidence:* `no-request-authentication.md` Instrumentation note ("always fire …
  low-value to instrument"); Low/latent tags on pubkey-representation,
  session-public-key-parse, guardian-routes-time-independent.
- *Suggested action:* Demote `no-request-authentication` to a P2 cross-reference;
  consider trimming/merging the Low cluster to refocus budget on attestation/sig.

**F-FIND-10 — [Property pair: key-write-durable-or-rejected + torn-read-never-yields-wrong-key; + harness CVM mock]**
- *Concern:* (a) The crash-face and concurrency-face properties share root cause
  and outcome; one passes vacuously if its fault class is disabled. (b) The
  topology's CVM mock signs `keccak256` by construction, so
  `rotate-key-preimage-matches-onchain` can never observe the real-agent drift it
  was created to catch.
- *Scope:* two property pairs + topology interaction.
- *Evidence:* relationships doc Cluster 6 ("a fix closes both");
  `deployment-topology.md:62` (mock signs raw keccak256); rotate-key property's
  "version-compatibility hazard" (`branch = "dev"` unpinned agent dep).
- *Suggested action:* Make the merge/keep decision explicit and conditioned on
  which faults are enabled; document that the rotate-key property's CVM-drift arm
  is inert under the mock (only the contract-side preimage arm is live).

**F-FIND-11 — [Property: torn-read / key-write zero-key — on-chain consequence]**
- *Concern:* The zero-key (`Fr::zero()`) signature share's consequence is asserted
  ("corrupts threshold signing") but never modeled at the aggregator. A zero key
  yields a publicly-computable share / identity public-key-share — possibly a
  forgery seam or possibly harmless; unknown.
- *Scope:* property (missing cross-boundary angle).
- *Evidence:* zero-key escalation in both evidence files; no property traces the
  share to GuardianModule/threshold math.
- *Suggested action:* Add an open question (resolvable in the guardian+contracts
  harness): does an identity-element / zero-key share get rejected, ignored, or
  predictably accepted by the threshold aggregator?

### Passes (things the catalog gets right that I verified and will not re-litigate)

- **P1–P4 are the correct gating facts**, and the honest "Reachable today /
  aspirational Always for the future" framing of the inert paths is exactly right
  given the uncertainty.
- **The W0 code-level drift is real and accurately characterized** (verified
  firsthand: guardian 7-field at `mod.rs:322-336`; `feat/tdx` contract 5-field at
  `LibGuardianMessages.sol:32`; field order otherwise matches).
- **The trust-boundary reframe onto reef + contracts is correct** (coral confirmed
  not to call the guardian; reef is the sole consumer) — my refinement (ingestor
  above reef) extends it rather than contradicts it.
- **`stored-share-matches-committed-pubkey-share` (S4) is correctly identified as
  the strongest holdable invariant** and the load-bearing safety barrier.
- **The deployment-topology's three-container design (workload + guardian + anvil-
  with-real-contracts) is the right system-level harness** and already anticipates
  most of the framing critique in directive 1.
- **The fault-enablement caveats** (node-termination off by default; clock jitter
  as negative control; CVM unix socket not network-faultable) are accurate and
  important.

### Uncertainties (could not fully resolve in-repo; flagged for human / on-chain)

- **U1.** Whether the deployed GuardianModule is behind an upgradeable proxy later
  pointed at 7-field bytecode. Resolvable by `eth_getCode`/`cast call` against the
  pinned Hoodi/mainnet addresses (F-FIND-1) — I confirmed the *initial* Hoodi
  deploy was 5-field but cannot see post-deploy upgrades from static repos.
- **U2.** Whether puffer-ingestor's inbound endpoints (the real soft edge) require
  authentication. The agent found no documented inbound auth and Lambda/cron
  triggers, but I did not exhaustively audit ingestor's router middleware.
- **U3.** Whether beacon-chain voluntary exits require current-epoch (making
  epoch-0 a live correctness bug, F-FIND-8) — a consensus-layer/product question.
- **U4.** The production M-of-N and guardian count (catalog OQ2) — sets blast
  radius; genuinely needs human input.
- **U5.** Whether a zero-key/identity share is exploitable at the on-chain
  aggregator (F-FIND-11) — answerable only in the guardian+contracts harness.
- **U6.** Deployment topology centralized-vs-edge for reef-guardian: the agent
  found a shared-DB multi-instance pattern but no definitive per-guardian-vs-
  central answer; affects whether reef→guardian is genuinely one-private-link.

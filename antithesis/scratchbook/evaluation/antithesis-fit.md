---
sut_path: /home/fawad/puffer/projects/validator-guardian
commit: f7dbd88d99de21a6d8caba5f7de8216624ab1098
updated: 2026-06-30
external_references:
  - path: /home/fawad/puffer/projects/reef
    why: consumer call patterns referenced by the catalog
  - path: /home/fawad/puffer/projects/coral
    why: confirms coral does not call the guardian
---

# Antithesis Fit Evaluation — `guardian` binary property catalog

Lens: does each property require exploring a state space a unit/integration test
can't reach (timing/race, concurrency, partial failure, crash/restart,
combinatorial fault interleavings)? If a fixed-input unit test fully verifies it,
it is consuming Antithesis search budget that belongs to fault-sensitive
properties. The assessment is adversarial: it looks for catalog-as-portfolio
problems, not for confirmation.

Verified against source before writing: reef sends `verify_session: false`
(`new_registration.rs:266`, grep-confirmed); coral makes no guardian HTTP calls
(no `validate-custody`/`sign-exit`/`eth/v1/keygen`/guardian references in coral
src — grep-confirmed). Catalog = 27 properties, 27 evidence files (counts match).

---

## Findings

### Catalog-wide

#### CW1 — The catalog is meaningfully over-weighted toward workload-only / input-coverage properties relative to genuinely fault-timing-sensitive ones
- **Scope:** catalog-wide
- **Concern:** Of the 27 properties, only a minority *require* an Antithesis-only
  capability (deterministic interleaving, crash/restart, partition/hang) to be
  meaningfully verified. By my count, properties whose core value is
  fault/timing-sensitive and not reproducible by a deterministic test:
  **torn-read-never-yields-wrong-key, key-write-durable-or-rejected,
  no-orphaned-unacknowledged-secret, missing-state-request-fails-clean (restart
  face), dependency-hang-makes-progress, upcheck-live-under-load** — roughly 6,
  plus **malformed-input-never-panics** and **bounded-resource-usage** which gain
  a real edge from concurrency/scheduler exploration. The remaining ~13 are
  fundamentally deterministic-correctness or input-coverage properties whose
  evidence files themselves say so verbatim: custody-preimage ("Primarily
  input-coverage + regression detection, not fault injection"; "mostly a
  deterministic-correctness property, not a fault-injection one"),
  rotate-key-preimage ("Mostly deterministic-correctness / input-coverage … NOT
  fault injection"), custody-rejects-invalid-deposit-data ("largely
  unit-testable in isolation"), session-public-key-parse ("Not fault-driven; this
  is fuzzing the API shape boundary"), stored-share-matches ("primarily a
  correctness-under-input-variation property"), pubkey-representation
  ("Coverage + regression"), guardian-routes-time-independent (a negative-space
  tripwire), approval-deterministic (a pure-function replay anchor). That is a
  catalog whose center of mass is "differential/fuzz oracle a forked-chain or
  property test could host," not "fault interleaving." A 27-property catalog is
  defensible, but the *priority/budget signal* should make explicit that the
  Critical/High labels are dominated by **correlated-liveness** (preimage drift)
  and **security-documentation** (attestation-off chain) properties, not by
  Antithesis's differentiated strengths.
- **Evidence:** the per-file "honesty notes" listed above; investigation pass
  headlines in `property-catalog.md:96-101`.
- **Suggested action:** Add a portfolio-level "Antithesis leverage" tag per
  property (fault-essential / fault-amplified / fault-irrelevant) so search budget
  isn't spread evenly. Consider hosting the pure differential-oracle properties
  (the three preimage/representation ones) as a cheap always-on regression test
  *outside* the Antithesis run, and reserve the Antithesis budget for the ~8
  fault-essential/amplified properties plus the cross-cutting catastrophe chain.

#### CW2 — The "framed as Reachable today because attestation is OFF (P1)" decision is correct, but its honesty cuts the other way for budget: those properties produce near-zero Antithesis signal in the default harness
- **Scope:** catalog-wide (Category A entirely; the verify-path halves of Category B)
- **Concern:** attestation-verified-before-approval, session-verification-fails-closed,
  attestation-payload-reconstruction-matches, session-public-key-parse, the
  rotate-key on-chain verify, and the F7 panic are all gated on
  `verify_session=true`, which **reef never sends**. The catalog frames them as
  `Reachable` documenting-properties for *today* plus aspirational `Always` for
  *if attestation is turned on*. Using `Reachable` here is the right call (an
  `Always(approval ⇒ verified)` would correctly fail on the real workload — see
  attestation-verified-before-approval.md:39-52) and is NOT a dodge. The problem
  is downstream: a `Reachable("verify_session=false path taken")` is satisfied by
  the very first workload request and then contributes nothing for the rest of the
  run — it is a one-shot coverage tick, not an invariant Antithesis can stress.
  The genuinely interesting verify-path properties (fail-closed under RPC
  partition/hang/garbage) only become live if the workload itself drives
  `verify_session=true` against a mock/anvil SessionRegistry — which the topology
  doc plans, but which means their value is contingent on a workload knob, not on
  the SUT-as-shipped.
- **Evidence:** P1 (`property-catalog.md:37-43`); reef `verify_session:false`
  confirmed; session-verification-fails-closed marked "inert until
  verify_session=true" (`:137`); per-file scope caveats.
- **Suggested action:** Keep the `Reachable` framing, but explicitly label these
  as "workload-must-flip-verify_session-on to get any invariant signal" and make
  session-verification-fails-closed (fail-closed under RPC fault) the carrier of
  the real Antithesis value in Category A — it is the only attestation property
  that exercises a partial-failure interleaving. Demote the pure `Reachable`
  coverage ticks to one combined "attestation-off bypass is reachable" witness
  rather than four near-duplicate ones.

#### CW3 — Crash/restart-dependent properties silently pass vacuously unless node-termination is enabled, and the catalog's flagging is correct but the portfolio risk is concentrated
- **Scope:** catalog-wide (Category F)
- **Concern:** key-write-durable-or-rejected, no-orphaned-unacknowledged-secret
  (crash variant), missing-state-request-fails-clean (restart variant), and the
  writer×crash face of torn-read all REQUIRE node-termination, which is off by
  default on most tenants. The catalog flags this (`:57-64`). But this means the
  single highest-Antithesis-leverage cluster (the only properties that genuinely
  need crash injection) is exactly the one most likely to pass vacuously if the
  tenant config isn't changed — and three of them are High priority. If the harness
  ships without confirming termination is enabled, the catalog's most
  Antithesis-justified properties become no-ops while the deterministic-oracle
  properties keep "passing" and create a false sense of coverage.
- **Evidence:** `property-catalog.md:57-64`; key-write-durable "requires
  node-termination" (`:356`); torn-read merged file "Honest assessment"
  (`torn-read…md:274-279`).
- **Suggested action:** Make node-termination a hard precondition for shipping the
  Category-F properties (gate them behind a setup check that fails loudly if
  termination is unavailable), rather than letting them register as silent passes.

#### CW4 — Several SUT-side invariants are described but their assertions cannot be evaluated without instrumentation that does not yet exist, and the master lifecycle invariant is expected-to-fail
- **Scope:** catalog-wide
- **Concern:** The most valuable assertions in the catalog (the w7-6 master
  invariant "every signed exit came from a verified+approved share", the
  torn-read "loaded share's pubkey == requested share" guard, the
  sign-exit-index-binding store-index==read-index check, stored-share S4 mirror)
  all require **SUT-side instrumentation that records per-share metadata the code
  does not currently keep** (no approval marker on disk, no recorded decrypt
  index — confirmed in sign-exit-requires-authorization.md:95-102 and
  sign-exit-index-binding.md:166-169). Two consequences for fit: (a) these are the
  properties where Antithesis adds the most value, yet they are the most expensive
  to instrument and depend on adding side-channels to the SUT; (b) the master
  invariant is explicitly *expected to fail today* (attestation off), so it must
  ship as a `Reachable`/`Sometimes` witness, not an `Always`, or it will fire on
  every run and train the reader to ignore it.
- **Evidence:** attestation-verified-before-approval.md:140-145 ("This is the
  master assertion; it will FAIL today"); existing-assertions.md (nothing
  instrumented; SDK not a dependency).
- **Suggested action:** Sequence instrumentation so the side-channel metadata
  (decrypt index + approval marker per share) lands first — it unlocks
  sign-exit-index-binding, the w7-6 master witness, and the torn-read guard at
  once. Until then, mark these as "blocked on SUT instrumentation" so they are not
  counted as live coverage.

### Property-specific

#### P1 — `guardian-routes-time-independent`: the clock-jitter framing overstates Antithesis relevance; this is a near-pure negative-space tripwire
- **Property:** guardian-routes-time-independent
- **Scope:** property-specific
- **Concern:** The property asserts signed digests don't change under clock jitter.
  But the evidence file establishes (correctly) that the guardian reads no
  wall-clock anywhere on the four routes — exit epoch is a hardcoded `0`, no
  `tokio::time`/`Instant`/`SystemTime`. So the clock-jitter fault has *nothing to
  perturb*: the `Always(identical digest under clock step)` is true by construction
  and a one-line static audit (grep) verifies it more cheaply than a fault
  campaign. Spending clock-jitter search budget here yields no information; the
  fault is, as the catalog itself says, only a "negative control." The genuinely
  useful piece is the `Unreachable("a guardian route read SystemTime/Instant")`
  regression tripwire — which is a build-time/coverage assertion, not a
  fault-driven one.
- **Evidence:** guardian-routes-time-independent.md:14-32 (audit shows zero time
  sources); `property-catalog.md:520-527` (Priority Low, "negative-space
  invariant").
- **Suggested action:** Keep as a Low cheap tripwire (the `Unreachable`
  time-source guard) but explicitly de-scope the clock-jitter *fault* from this
  property's run config — it cannot fail and burns scheduler budget. It is
  correctly Low; just make clear it earns nothing from fault injection.

#### P2 — `approval-deterministic-and-idempotent`: the determinism `Always` is a pure-function property a unit test settles; only the crash-then-retry variant is Antithesis territory
- **Property:** approval-deterministic-and-idempotent
- **Scope:** property-specific
- **Concern:** "Identical request ⇒ identical approval bytes" is, post-investigation,
  *confirmed by construction* (RFC-6979 deterministic k, low-S, pure preimage —
  evidence file Investigation Log). A two-call unit test proves it. Antithesis adds
  value only in the **crash-between-share-write-and-response → reef-retry → re-write
  must be byte-identical** scenario (which needs node-termination) and the
  **concurrent duplicate-submit** scenario (which needs the scheduler). The catalog
  files this at Medium and does flag the crash/concurrency angle, which is right —
  but the headline `Always(identical sig)` is the least Antithesis-relevant part
  and should not be what justifies the property's place in the run.
- **Evidence:** approval-deterministic-and-idempotent.md:117-154 (determinism fully
  resolved as by-construction); Antithesis angle pins the value on crash-retry +
  concurrency (`:77-89`).
- **Suggested action:** Reframe the property's Antithesis justification around the
  crash-retry/concurrent-write interleaving (which overlaps torn-read) and treat
  the determinism `Always` as a free regression sentinel riding along, not the
  reason to spend budget. Note the dependency on node-termination for the crash
  half (same gate as CW3).

#### P3 — `custody-preimage-matches-onchain-verifier` / `rotate-key-preimage-matches-onchain`: correctly Critical/High, but the value is a differential oracle, not Antithesis fault search — and they need an out-of-process verifier that is an environment dependency
- **Properties:** custody-preimage-matches-onchain-verifier, rotate-key-preimage-matches-onchain
- **Scope:** property-specific
- **Concern:** The W0 drift is the single highest-severity finding (a 7-field Rust
  preimage vs a 5-field deployed verifier ⇒ protocol-wide provisioning stall), and
  it absolutely belongs in the catalog. But its truth lives in a contract that is
  *not in the SUT*; verifying it requires a reference encoder or a forked-chain
  GuardianModule in the harness. Antithesis's contribution is reduced to
  input-space diversity over `(chainId, validatorIndex, address, pubkey/sig/wc
  lengths)` plus catching a *future* recurrence — exactly what a foundry
  differential test or proptest does, deterministically and faster. The real
  resolution (which bytecode is deployed) is human-input, not something the
  scheduler can discover. So these are high-*severity* but low-*Antithesis-fit*: the
  bug is real and the property is justified, but it is mis-cast if presented as a
  reason Antithesis is the right tool. The genuine Antithesis-only angle nearby —
  version-skew / mixed-binary guardians signing different field counts mid-rolling-
  upgrade (in the merged coordination file) — is buried as a `Sometimes` and would
  need a multi-guardian topology the deployment doc explicitly does not recommend
  for the first harness.
- **Evidence:** custody-preimage…md:78-98 ("Best expressed as an oracle/differential
  test … mostly a deterministic-correctness property, not a fault-injection one");
  rotate-key…md:82-91; deployment-topology.md:48 (3-guardian variant "not
  recommended for the first harness").
- **Suggested action:** Keep at Critical/High for severity, but tag explicitly as
  "differential-oracle, Antithesis adds input-coverage + regression only." If the
  anvil+contracts topology is built, ensure the `Sometimes(produce→verify
  round-trip succeeds)` is in the run so a *misaligned* deployment is caught as an
  unreachability — that is the one piece the scheduler genuinely helps with.

#### P4 — `custody-rejects-invalid-deposit-data` and `session-public-key-parse-no-wrong-identity`: honestly flagged as unit-test territory — recommend demotion/folding, not standalone budget
- **Properties:** custody-rejects-invalid-deposit-data, session-public-key-parse-no-wrong-identity
- **Scope:** property-specific
- **Concern:** Both are self-described as not fault-driven. custody-rejects
  (S1–S3) is "largely unit-testable in isolation … Antithesis's added value here
  is modest." session-public-key-parse was *already downgraded to Low* and is
  liveness-only (the on-chain verifier fails closed on a bad typeId, confirmed),
  AND the JSON branch is dead for SUT-originated payloads (the in-repo producer
  emits raw hex), so the property is fuzzing a branch that no real producer reaches.
  These are correct as *guardrails* but should not consume distinct Antithesis
  search budget: S1–S3 only becomes interesting when combined with
  fork-version-source-consistency (caller-chosen signing domain) and the `.zip`
  truncation, and the session-key parse is essentially dead code today.
- **Evidence:** custody-rejects-invalid-deposit-data.md:46-52 ("honesty note");
  session-public-key-parse…md:7 (downgraded, liveness-only) and :96-108 (JSON branch
  dead for SUT payloads).
- **Suggested action:** Fold custody-rejects into the `Always` guardrail of
  stored-share-matches / fork-version-source-consistency rather than running it as
  a standalone input-fuzz target. Demote session-public-key-parse to a
  documented-but-deprioritized Low (or drop from the first harness) given the JSON
  branch is unreachable for real payloads; keep only the cheap `Always(returned
  typeId is supported)` sentinel.

#### P5 — `pubkey-representation-consistent`: latent-not-live by the catalog's own investigation; over-valued as a standalone property
- **Property:** pubkey-representation-consistent
- **Scope:** property-specific
- **Concern:** The investigation downgraded this to "latent, not live today" —
  reef round-trips the uncompressed form and shares the `ecies` type via the git
  dep, so the compressed/uncompressed/base64 three-faces split does not bite. It is
  a regression tripwire for *future* encoding drift, asserted as a round-trip
  equality the workload can check once. There is no fault, timing, or concurrency
  dimension; a serde round-trip unit test is the natural home.
- **Evidence:** `property-catalog.md:497-502`; pubkey-representation evidence
  resolution ("latent fragility … assert the round-trip as a tripwire").
- **Suggested action:** Keep as a Medium *tripwire* but host it as a workload
  round-trip assertion that fires once, not as an ongoing Antithesis target. It is
  fault-irrelevant; do not let it count toward fault coverage.

#### P6 — `no-request-authentication`: correctly a documenting `Reachable`, but it is a single one-shot tick, not a property that benefits from search
- **Property:** no-request-authentication
- **Scope:** property-specific
- **Concern:** This is a threat-model fact ("privileged endpoint served an
  unauthenticated request") asserted as `Reachable`. The very first workload
  request satisfies it forever. It is correctly Low and correctly framed, but it is
  pure documentation — Antithesis neither stresses nor can falsify it. Its real
  function is to gate the *severity* of other properties (via Q4), which is a
  catalog-reading aid, not a runnable property.
- **Evidence:** `property-catalog.md:532-544`.
- **Suggested action:** Keep as a one-line `Reachable` witness; do not treat it as
  occupying a property "slot" for coverage purposes. Its value is the Q4 severity
  gate, which belongs in the preconditions section (it largely already is, as P2).

#### P7 — `sign-exit-index-binding`: under-celebrated — this is a high-Antithesis-fit cross-endpoint binding bug, not just a config corner
- **Property:** sign-exit-index-binding
- **Scope:** property-specific
- **Concern (value underestimated, in the opposite direction):** This is one of the
  strongest Antithesis-fit properties in the catalog and reads as merely "High". It
  is a confirmed silent cross-endpoint desync (custody stores by intrinsic decrypt
  index i; sign-exit reads by reef's hardcoded `guardian_index=0`) that *only*
  surfaces when the workload drives a guardian whose share decrypts at i≠0 and then
  exercises sign-exit — i.e. it needs the scheduler/workload to explore the
  index-space and the cross-endpoint sequence, which a single fixed unit test won't
  do (the SUT's own `test_sign_vem` loops indices and thus *masks* the bug by always
  matching i to guardian_index). The harm (a guardian silently mute on exits →
  set can't reach threshold to eject → funds stuck) is latent until an emergency.
  This is exactly the "deterministic test can't reach it" sweet spot.
- **Evidence:** sign-exit-index-binding.md (full); reef hardcoded
  `guardian_index=0` confirmed (`eject_validator.rs:170`); SUT test loops indices
  (masking, `:147-153`).
- **Suggested action:** Elevate this in the run priority (it is arguably the best
  pure-Antithesis-fit *correctness* property after the concurrency cluster) and
  ensure the workload deliberately configures the guardian at a non-zero index. It
  needs the SUT-side decrypt-index side-channel from CW4.

#### P8 — `cvm-stub-never-in-production`: a config/reachability check with no Antithesis-search dimension
- **Property:** cvm-stub-never-in-production
- **Scope:** property-specific
- **Concern:** Toggling `CVM_AGENT_STUB` and asserting a 201 can carry empty
  evidence is a config-driven boolean — there is no timing, race, or fault
  interleaving. The "must never reach a production image" half is a build-audit
  question (human input, Q5), not something the scheduler explores. It is correctly
  Medium but is environment/config testing wearing a property's clothes.
- **Evidence:** `property-catalog.md:145-157`; cvm-stub evidence (config toggle).
- **Suggested action:** Keep as a Medium config-reachability witness but mark it
  fault-irrelevant; the real mitigation (strip the stub from prod builds) is a CI
  check, and the catalog should say so rather than implying Antithesis verifies it.

#### P9 — `dependency-hang-makes-progress`: genuine high-fit liveness property, but today inert for the RPC half and partly testable only via the in-container mock
- **Property:** dependency-hang-makes-progress
- **Scope:** property-specific
- **Concern (mostly a pass with a caveat):** The CVM-agent-hang variant is strong
  Antithesis territory (hang the socket, `ANTITHESIS_STOP_FAULTS`, assert
  progress) and confirmed reef has no client timeout, so the harm is real. But two
  scoping facts dull it: (a) the SessionRegistry-RPC-hang half is only live on the
  `verify_session=true` path (off in prod), and (b) the CVM agent is a unix socket
  that Antithesis network faults *cannot reach* — its hang must be driven by the
  in-container controllable mock, not by Antithesis's partition/latency machinery.
  So the property's Antithesis-native fault (network partition) applies only to the
  RPC half, which is inert today; the live half relies on a custom mock control
  file. Worth stating plainly so the harness builds the mock's `hang` mode.
- **Evidence:** dependency-hang evidence; deployment-topology.md:50-73 (unix socket
  not network-faultable; mock `hang` mode required); RPC-hang "only live when
  verify_session=true".
- **Suggested action:** Keep at High; flag that the live (CVM) half needs the mock's
  hang mode (not an Antithesis network fault) and the network-fault-native (RPC)
  half needs `verify_session=true`. This is a topology/workload dependency, not a
  property defect.

#### P10 — `startup-bind-failure-not-silent`: real bug, but the bind-discard is a deterministic startup fact; Antithesis adds little beyond restart-into-bad-config
- **Property:** startup-bind-failure-not-silent
- **Scope:** property-specific
- **Concern:** The discarded `axum::Server::bind` result and unconditional 200
  `/upcheck` are real defects, but "start with a held port → process exits silently"
  is a deterministic integration test (occupy the port, launch, observe exit). The
  Antithesis-only slice is "restart onto a bad config / dead CVM socket via
  node-termination and observe `/upcheck` still 200" — which overlaps
  missing-state-request-fails-clean and again needs termination (CW3). Medium is
  right; just don't over-credit fault search.
- **Evidence:** `property-catalog.md:476-488`; needs node-termination for the
  interesting variant.
- **Suggested action:** Keep Medium; note the readiness-vs-liveness `/upcheck`
  defect is verifiable deterministically and only the restart-into-bad-config
  variant is Antithesis-specific.

---

## Passes (assessed and look right)

- **Concurrency cluster is correctly the Antithesis core.**
  torn-read-never-yields-wrong-key is the model property for this tool: a
  non-atomic O_TRUNC write racing a read, with a *confirmed* fail-open to a
  blsttc zero-key (HTTP 200 + wrong signature), reachable only inside a
  truncate→write window that thread-pause/CPU-modulation can deterministically
  hit. Assertion choice (`Unreachable(torn/zero-key signed)` +
  `Reachable(torn read observed)`) is exactly right, and the escalation from
  "fails closed but flaky" to a safety break is well-evidenced in blsttc source.
- **upcheck-live-under-load** is a genuine head-of-line-starvation property
  (blocking fs/ECIES/BLS on async workers, no `spawn_blocking`, uncapped
  caller-controlled N in the trial-decrypt loop). Liveness framing
  (`Sometimes` under load + `eventually_` recovery after `ANTITHESIS_STOP_FAULTS`)
  fits, and it needs only node-throttle/CPU-mod, not termination — a cheap,
  high-fit property.
- **`AlwaysOrUnreachable` vs `Always` choices are sound** where used:
  key-write-durable-or-rejected uses `Unreachable` for the BLS zero-key branch and
  `AlwaysOrUnreachable` for the ETH branch (correct — the ETH path can be
  legitimately unreachable in a run with no eth-key read); approval-deterministic
  mirrors the existing `bail!` self-recover as `AlwaysOrUnreachable` (right — only
  reachable if custody runs). No `Always` was found that should obviously be
  `AlwaysOrUnreachable` except where the catalog already chose AOU.
- **The decision to use `Reachable` for the attestation-off and sign-exit-no-auth
  gaps is honest, not a dodge.** An `Always(approval ⇒ verified)` or
  `Always(exit ⇒ authorized)` would fail on the real workload / has no "authorized"
  notion in code; the catalog says so explicitly and ships them as
  `Reachable`/`Sometimes` witnesses with aspirational `Always` for the future. That
  is the correct SDK usage for documenting an accepted-but-risky boundary.
- **stored-share-matches-committed-pubkey-share** is correctly identified as the
  strongest holdable invariant, encoded as `Unreachable(stored non-matching share)`
  — the right assertion type for a barrier that must never be crossed; and it is
  paired with a `Sometimes(reject branch exercised)` so the Unreachable isn't
  vacuous. Good discipline.
- **no-orphaned-unacknowledged-secret** correctly separates the crash variant
  (needs termination) from a fault-free error-after-write variant, so it yields
  *some* signal even without termination.
- **The cross-cutting "catastrophe chain"** (attestation-off + orphaned share +
  ungated sign-exit, property-relationships.md:129-134) is the single best
  end-to-end Antithesis scenario in the catalog — it composes three local facts
  into a timeline only a multi-step fault+workload sequence reaches. Keeping it as
  one workload scenario is the right framing.

## Uncertainties (could not determine, and why)

- **Whether the fault-essential cluster will actually run.** node-termination
  enablement is a tenant config fact (Antithesis side), not in this repo; I cannot
  confirm it will be on, so I cannot confirm CW3's properties will produce signal
  rather than vacuous passes. Flagged as the dominant portfolio risk.
- **Production M-of-N and guardian-index distribution** (Q2 / sign-exit-index-binding
  open Q): how often a real guardian lands at index ≠ 0 determines how
  high-frequency the index-binding bug is. Human input.
- **Which `puffer-contracts` bytecode is deployed** (Q1): governs whether
  custody-preimage is a *live* protocol-stall today or a latent regression
  tripwire. Code-level drift is confirmed; deployment is human input. This changes
  the property's severity but not its (low) Antithesis-fit.
- **Whether `verify_session` will be flipped on** (Q3): governs whether the entire
  Category-A `Reachable` set ever upgrades to live `Always` invariants worth
  fault-stressing, or stays one-shot coverage ticks.
- **Q4 (untrusted reachability of validate-custody/sign-exit):** I could confirm
  coral doesn't call the guardian and reef is the sole consumer, but not the TDX
  network topology — which sets whether malformed-input-never-panics and
  sign-exit-requires-authorization are remote-DoS/force-eject primitives (High) or
  defense-in-depth (Medium). Pure human/ops input; it shifts priorities but not the
  Antithesis-fit reasoning.
- **Real DCAP CVM agent byte behavior** (rotate-key open Q): the production signer
  is closed-source and absent from all three available repos; the sim agent is
  byte-verified but the property's cross-process correctness against the real agent
  cannot be settled here. Does not change the (differential-oracle) fit assessment.

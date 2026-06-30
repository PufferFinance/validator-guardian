# Property: sign-exit-replayable-without-authorization

Focus: (9) Idempotency & Replay (unauthorized replay of an exit-signing capability)
Slug / canonical ID: `sign-exit-replayable-without-authorization`

## One-sentence property
`sign-exit` deterministically mints a voluntary-exit signature share for **any**
caller-supplied `(bls_pub_key_set, guardian_index, validator_index, fork_info)` for
which a BLS share is on disk, with **no authorization, custody, or attestation gate**
and a **hardcoded epoch 0** — so the same exit share is reproducible/replayable
indefinitely; the property asserts the deterministic, ungated nature so the dangerous
state is observable and any future gating is detectable.

## What led to this property
- W10 (SUT §9): "`sign-exit` has zero authorization. It mints exit-signature shares
  for any caller-supplied `(blsPubKeySet, guardian_index, validator_index)` with a
  stored share, epoch hardcoded to 0, no custody/ownership/attestation gate. Combined
  with the inert attestation + write-before-success persistence, an
  unattested-then-persisted share becomes a live exit-signing capability."
- SUT §13(d): "unauthorized/forced `sign-exit` reaching M guardians → **force-ejects
  live validators**." This is the highest-severity *active-harm* outcome in the system.
- F9 (SUT §3/§7): orphaned shares persist before approval and on failure → a share
  can exist on disk that was never successfully approved, yet sign-exit will still use
  it.

## Code evidence (files + functions + lines)
- `src/enclave/guardian/mod.rs:352-370` `sign_voluntary_exit_message`:
  - `:356-360` looks up the share file by
    `pk_set.public_key_share(req.guardian_index)` — `guardian_index`,
    `validator_index`, `fork_info`, and `bls_pub_key_set` are **all from the request**
    (`SignExitRequest`, `types.rs:229-243`). No ownership/attestation check.
  - `:361` `fetch_bls_sk(&pk_hex)?` — succeeds for any share present on disk.
  - `:364` `sign_vem(sk, 0, ...)` — **epoch hardcoded to 0** (`:363` comment "Epoch
    0").
- Signing is deterministic: `sign_vem` (`mod.rs:372-394`) computes a fixed signing
  root from `(fork_info, voluntary_exit{epoch:0, validator_index})` and signs with the
  stored share → same inputs ⇒ same signature bytes (BLS signing is deterministic).
- Handler has no auth layer: `src/enclave/guardian/handlers/sign_exit.rs:5-21` is a
  bare `Json(request)` → call. No middleware on the router (SUT §2: "Middleware:
  none").
- Persistence-before-approval makes shares available early: write at
  `mod.rs:82-85` happens before `approve_custody` (`:88`).

## What breaks / what this documents
- **Replay / forced ejection:** an exit signature share, once produced, is valid
  forever for `(validator_index, epoch=0, fork)`; there is no nonce, deadline, or
  used-marker. Any party that can reach the endpoint and knows the public
  `bls_pub_key_set` + a `validator_index` can obtain a guardian's exit share. If ≥ M
  guardians serve this (all run the same binary), the shares aggregate into a valid
  voluntary-exit that **force-ejects a live validator** (SUT §13d).
- **Determinism is the replay enabler:** because the output is a pure function of
  request fields + stored share, retries/replays are byte-identical — useful as a
  property anchor and as the thing that makes the capability "live".

## Suggested assertion(s) and types
1. **`Reachable`** / SUT-side marker at the top of `sign_voluntary_exit_message`:
   record that an exit signature was produced with **no authorization gate** and
   `epoch == 0`. Message: "sign-exit produced an exit share with no custody/ownership
   check (epoch=0)". This makes the W10 capability a first-class searchable outcome.
   Type rationale: exploration hint exposing a dangerous-but-currently-intended path.
   **Instrumentation: MISSING.**
2. **`AlwaysOrUnreachable`** (SUT-side) asserting epoch is always 0 at the
   `sign_vem(sk, 0, ...)` call — a tripwire: if a future change makes epoch
   caller-controlled, the assertion (epoch==0) breaks, flagging that exit messages can
   now target arbitrary epochs (a different replay surface). Type rationale: holds on
   every sign-exit; unreachable if none occur. **Instrumentation: MISSING.**
3. **`Always`** (workload-side): call `sign-exit` twice with identical inputs; assert
   byte-identical `signature` — confirms deterministic replayability (the property's
   core claim). **Instrumentation: workload-only.**
4. **`Sometimes`** that `sign-exit` succeeds for a share that was persisted by a
   `validate-custody` whose approval step **failed/never completed** (orphaned share,
   F9). Message: "sign-exit signed using an orphaned (unapproved) BLS share". This ties
   replay to the write-before-success gap and is the most damaging concrete scenario.
   Type rationale: a meaningful, dangerous state that should be shown to occur ≥ once.
   **Instrumentation: MISSING** (needs SUT-side signal distinguishing approved vs
   orphaned shares — there is currently no approval marker on disk; see open
   questions).

## Antithesis angle (faults / timing / interleaving)
- **Crash between share-write and approval (F9):** inject a container restart / error
  after `write_bls_key` (`mod.rs:85`) but before/within `approve_custody`. Then issue
  `sign-exit` for that share. Assertion (4) fires → demonstrates an unattested,
  unapproved share is exit-signable.
- **Concurrent custody + sign-exit on the same share** (SUT §4 torn read): exercises
  both the data-integrity torn-read and the replay capability simultaneously.
- **No fault even needed for (1)/(3):** the ungated/deterministic nature is reachable
  on a clean run; faults make the orphaned-share variant (4) concrete.

## Timing / config dependencies
- Requires a BLS share on disk (prior `validate-custody`, hence prior keygen).
- Epoch-0 hardcoding is config-independent. Determinism independent of clock (good
  negative control under clock jitter).

## Open questions
- **Is there any on-disk/in-memory marker that a share was successfully approved?**
  Searched: none found (write happens, approval is a separate later step with no
  persistence of its outcome). *Why it matters:* assertion (4) needs to distinguish
  "approved" from "orphaned" shares; without a marker it must be inferred from the
  workload's own bookkeeping (which custody call failed). *What changes:* if a marker
  is added (recommended fix), (4) becomes a clean SUT-side `Sometimes`; without it, the
  workload must track approval outcomes externally. **Needs design input (also Open Q6
  in SUT: any GC/reconciliation for orphaned files? — none found).**
- **Is `/guardian/v1/sign-exit` reachable by untrusted clients?** (mirrors Open Q4.)
  *Why it matters:* decides whether W10 is a remote force-eject primitive or
  defense-in-depth. *What changes:* if remotely reachable, this is the single
  highest-severity property in the catalog (active fund/validator harm) → High;
  otherwise it documents a missing internal authz gate. **Needs human input on TDX
  topology.**
- **Should `sign-exit` require proof of custody/attestation for the validator being
  exited?** *Why it matters:* if product intends exits to be gated, assertion (1)'s
  "no authorization gate" marker becomes a defect signal and (2)'s epoch tripwire pairs
  with a new authz tripwire. *What changes:* converts the `Reachable` markers into
  `Unreachable`/`Always` safety invariants. **Needs product input.**


---

> **[merged]** consolidated from discovery focus file `prop-focus-5/sign-exit-authorization.md`

# sec-sign-exit-authorization — Exit signature minted with zero authorization

## Origin
Focus (6) Security Boundaries; SUT-analysis W10, §13(d); lens lead W10.

## Files / functions / lines
- `src/enclave/guardian/mod.rs:352-370` `sign_voluntary_exit_message`:
  - `:356-360` derives the pubkey-share filename purely from caller-supplied
    `req.public_key_set()` + caller-supplied `req.guardian_index`.
  - `:361` `fetch_bls_sk(&pk_hex)` — reads whatever share is stored at that filename.
  - `:364` `sign_vem(sk, 0, req.validator_index, req.fork_info)` — epoch **hardcoded 0**,
    `validator_index` taken straight from the request.
- `src/enclave/types.rs:229-243` `SignExitRequest { bls_pub_key_set, guardian_index,
  validator_index, fork_info }` — every field attacker-controllable.
- `src/enclave/guardian/handlers/sign_exit.rs:5-21` — handler does no auth.
- `src/crypto/bls_keys.rs:58-65` `fetch_bls_sk` — pure file read by pubkey-share hex,
  no ownership/identity check.
- Consumer: `/home/fawad/puffer/projects/reef/reef-guardian/src/handlers/api/webhooks/eject_validator.rs:162-205`
  `sign_vem` — reef builds the request from its own DB; guardian does not verify any of it.

## Precise trust assumption
The guardian assumes: *whoever calls `/guardian/v1/sign-exit` is authorized to eject
the validator whose share is requested.* There is **no** custody gate, no ownership
proof, no attestation, no nonce, no signature from an authorized party — the only
precondition is that a matching BLS share already exists on disk (which it will, for
every validator this guardian co-provisioned).

## Adversarial sequence that violates it
1. Any party reaching the port (no middleware) sends `SignExitRequest` with the
   `bls_pub_key_set` of a *live, healthy* validator and `guardian_index` matching
   this guardian's stored share, plus any `validator_index`.
2. Guardian returns a valid exit-signature share — no questions asked.
3. Repeat against >= M guardians (all run the same binary) -> reconstruct a threshold
   voluntary-exit signature -> **force-eject a live validator** that the operator
   never intended to exit.

## Real-world impact
Catastrophic availability/economic attack: forced ejection of live validators across
the set, draining/exiting stake against the operator's will. Combined with F1
(unattested-then-persisted share, see `approval-implies-session-verified.md`), an
attacker-introduced share becomes a live exit-signing capability.

## Honest framing (needs human judgment — likely by-design-but-risky)
The guardian as written has **no authorization on sign-exit by design** — it trusts
the reef/BFF orchestrator over a (presumed) private link. So a property like
`Always(exit_signature ⇒ caller_authorized)` is **currently UNENFORCEABLE** (there is
no notion of "authorized" in the code). Honest framings:
- Frame as a **property the system SHOULD have** but does not:
  `Unreachable("exit signature produced for a request that carried no authorization proof")`
  — would fire on essentially every call today, i.e. it documents the gap. Mark as
  expected-to-fail / Bias-Human-Input rather than asserting it's a live bug.
- Or, the testable *positive* fact:
  `Sometimes("sign-exit produced a signature with no custody/ownership/attestation gate present")`
  / `Reachable` — confirms the unauthenticated mint is reachable so the team must
  consciously accept the trust boundary or add a gate.

## Antithesis angle
Workload-driven: issue sign-exit requests for shares the "attacker workload" never
legitimately provisioned (after a validate-custody by a different logical actor) and
assert a signature still comes back. No fault injection needed to demonstrate the
authz gap, though clock faults can additionally probe the hardcoded-epoch-0 issue
(an exit signed at epoch 0 vs the chain's real epoch — separate correctness concern).

## Instrumentation
**MISSING.** Requires SUT-side instrumentation in `sign_voluntary_exit_message`
marking "signature produced" together with the (absent) authorization context. The
honest assertion is mostly a *documentation* assertion of an unenforced boundary.

## Open questions (why they matter)
- **Is the lack of any sign-exit authorization by design?** (the guardian trusts reef
  over a private link). Decides bug vs accepted-trust-boundary. **Needs human input.**
- **Is sign-exit reachable by untrusted clients in the TDX topology** (OQ#4)?
  Determines real exploitability.
- **Why epoch 0?** Hardcoded `0` may be intentional (consensus accepts past-epoch
  exits) or a latent bug — flag for the eth2 owner.
- **Production M-of-N** (OQ#2) sets the blast radius of a quorum-reaching attack.

## Investigation Log

#### Is `sign_voluntary_exit_message` a pure deterministic function of its request (infinitely replayable)?
- **Examined:**
  - `src/enclave/guardian/mod.rs:352-394` `sign_voluntary_exit_message` / `sign_vem`.
  - `src/crypto/bls_keys.rs:58-65` `fetch_bls_sk` (pure file read → `SecretKeySet::from_bytes`).
  - blsttc pinned source `~/.cargo/git/checkouts/blsttc-…/db34805/src/lib.rs`:
    `SecretKey::sign` (:431-434) → `sign_g2(hash_g2(msg))` (:426-429); `hash_g2`
    (:908-911) = `G2Projective::hash_to_curve(msg, DST, &[])`.
- **Found:** the signed root is `BLSSignMsg::VOLUNTARY_EXIT(...).to_signing_root()` over
  `(fork_info, VoluntaryExit{ epoch: 0, validator_index })` — every input is taken from
  the request (epoch hardcoded 0). BLS signing is `hash_to_curve(root)` (deterministic,
  no nonce) followed by `point · sk` (scalar mult). For a fixed request + fixed on-disk
  share, the output `signature`/`message` are **byte-identical** on every call. No
  randomness, no clock, no nonce anywhere in the path.
- **Not found:** no nonce/deadline/used-marker; nothing that would make two identical
  `sign-exit` calls diverge.
- **Conclusion:** **RESOLVED (determinism sub-point only).** `sign-exit` is a pure
  deterministic function of its request → the exit share is **infinitely replayable**
  byte-for-byte. This confirms assertion (3) ("call twice ⇒ byte-identical signature")
  is a sound workload anchor, and it is exactly what makes the ungated capability
  *live* (a captured share stays valid forever). The by-design/authorization and
  hardcoded-epoch-0 questions are **unchanged** and remain **needs human input** (this
  log resolves only the determinism claim, not the security questions).

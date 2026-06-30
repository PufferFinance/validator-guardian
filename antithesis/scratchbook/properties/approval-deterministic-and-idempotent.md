# Property: custody-approval-deterministic-under-retry

Focus: (9) Idempotency & Replay
Slug / canonical ID: `custody-approval-deterministic-under-retry`

## One-sentence property
Re-submitting an identical `validate-custody` request (same `keygen_payload`,
`guardian_module_address`, `chain_id`, `validator_index`, and an already-persisted
enclave ETH key) must return the **byte-identical** `enclave_signature` and persist
**byte-identical** share content — the operation is idempotent under retry.

## What led to this property
- Focus prompt: "Is `validate-custody` idempotent under retry (re-decrypt + re-write
  same content; is the approval ECDSA signature deterministic — ethers
  `LocalWallet::sign_message`, RFC6979? verify)?"
- reef retries are real: on a `validate_custody` error reef calls
  `skip_provisioning` (new_registration.rs ~316-330), and the whole
  provision-or-skip webhook can be re-delivered. The guardian write-before-success
  (SUT §3) means a crash after the share write but before the response yields an
  orphaned share (F9) that a retry will re-write — re-write **must** produce the same
  bytes, else two retries could leave divergent on-disk state.
- SUT §4 "Torn read": two `validate-custody` for the same share both `write_bls_key`
  to the same path — content is asserted "deterministic so no wrong-key outcome", a
  claim worth turning into an enforced invariant.

## Code evidence (files + functions + lines)
- Approval signing is **deterministic ECDSA**: `src/enclave/guardian/mod.rs:341-342`
  `wallet = ... .parse::<LocalWallet>()?; sig = wallet.sign_message(&msg).await?`.
  `ethers = "2.0.8"` (`Cargo.toml:50`); `LocalWallet` signs with k256 RFC6979
  deterministic nonces → for fixed `(sk, msg)` the signature is byte-stable. The
  preimage `msg` (`mod.rs:322-339`) is a pure function of request fields only
  (no timestamp, no nonce, no `session_id`).
- ETH key is fetched, not regenerated, on each call: `mod.rs:64-68`
  `fetch_eth_key(...)` — so the signing key is stable across retries.
- Share write content is deterministic: `mod.rs:82-85` writes
  `hex(sk_share.public_key_share().to_bytes())` (filename) and
  `hex(sk_share.to_bytes())` (content); `sk_share` is a deterministic function of the
  ciphertext + enclave sk via `decrypt_sk_share` (ECIES **decryption** is
  deterministic even though ECIES *encryption* is not).
- `write_key` is unconditional overwrite (`io/key_management.rs:9-14`) — a re-write
  with identical content is a no-op in effect (same bytes), but a re-write with
  *different* content would silently replace (no guard) — which is why determinism
  matters.

## What breaks if violated
- **Non-deterministic signature** would mean two reef-guardian retries submit two
  different `enclave_signature`s for the same `(validatorIndex, blsPubKey, …)`. The
  on-chain `GuardianModule` verifies a fixed preimage; a non-canonical/high-S or
  randomized signature could fail recovery on-chain → provisioning stalls
  (liveness). It would also break any caller-side dedup that keys on signature bytes.
- **Non-deterministic share content** under re-write (e.g. if the decrypt loop picked
  a different matching index on a different run) would leave the on-disk share
  different from the one the *first* approval committed to → later `sign-exit`
  produces shares inconsistent with the group → un-exitable validator (ties into
  `persisted-bls-share-roundtrips-or-rejected`).

## Suggested assertion(s) and types
1. **`Always`** (workload-side, no SUT change needed): the workload sends the same
   `validate-custody` twice and asserts the two responses are byte-equal
   (`enclave_signature` field). This is a black-box idempotency check. Type
   rationale: must hold on every duplicate-send the workload performs.
   **Instrumentation: present once the workload is written; SUT-side: not required.**
2. **`AlwaysOrUnreachable`** (SUT-side) inside `approve_custody` after `sign_message`:
   assert the signature is in canonical low-S form and recovers to `wallet.address()`
   (the recover check at `mod.rs:345-347` already exists as a `bail!`; mirror it as an
   Antithesis assertion so a failure is *reported* as a property violation rather than
   just a 500). Type rationale: every signing must satisfy it; "unreachable" only if
   no custody runs. **Instrumentation: partial** — the recover check exists as a
   `bail!` (HTTP 500), not as an Antithesis assertion; the canonical-low-S check is
   MISSING.
3. **`Sometimes`** that a duplicate `validate-custody` for an already-persisted share
   is processed (the re-write path actually executes), proving the idempotency case is
   exercised: message "validate-custody re-processed an already-stored BLS share".
   **Instrumentation: MISSING** (needs a SUT-side marker or workload-observable
   signal; the handler does not currently distinguish first-write from re-write).

## Antithesis angle (faults / timing / interleaving)
- **Crash-then-retry:** Antithesis container restart (or node termination if enabled)
  injected *between* the share write (`mod.rs:85`) and the HTTP response makes reef
  retry. Property (1) then checks the retry's signature matches what would have been
  returned — and `persisted-bls-share-roundtrips-or-rejected` checks the re-written
  share matches.
- **Concurrent duplicate submits:** fire two identical `validate-custody` concurrently
  (Antithesis interleaving / thread-pause). With no lock, both write the same path;
  the property asserts both responses are identical and the final file content is the
  expected bytes (rules out a torn final state being left behind).
- **Clock jitter** must NOT change the signature — since the preimage has no time
  component, jitter is a good negative control: the assertion should still hold.

## Timing / config dependencies
- Determinism hinges on RFC6979 in ethers `LocalWallet` / k256. **If a future bump
  changed the signing backend to randomized ECDSA, this property flips from "always
  true" to a real bug** — which is exactly why it is worth pinning.
- Requires the enclave ETH key to remain on disk across the two sends (no intervening
  keygen, which would mint a new key and is a different pubkey/file).

## Open questions
- ~~**Does ethers v2 `LocalWallet::sign_message` always emit canonical low-S?**~~
  **RESOLVED — yes, deterministic (RFC-6979) AND canonical low-S.** See Investigation
  Log below. The determinism premise of this property is CONFIRMED; assertion (2)'s
  low-S check is cheap insurance, not a standalone High-priority bug.
- **Is the ECIES ciphertext (`bls_enc_priv_key_shares`) ever re-generated by an
  upstream retry with fresh randomness?** ECIES encryption is non-deterministic
  (ephemeral key). *Why it matters:* if reef/coral re-encrypts the share on retry, the
  guardian receives a *different* ciphertext that still decrypts to the *same*
  plaintext share — so the guardian's output stays identical (good), but only if the
  decrypt-loop always lands on the same index. *What changes:* confirms the property
  holds across upstream re-encryption, strengthening confidence. The guardian itself
  never encrypts, so this is an upstream assumption to note, not a guardian bug.
  **(partial: guardian-side decrypt is deterministic — ECIES *decryption* is a pure
  function of (ciphertext, sk); confirmed in trial-decrypt evidence file. The
  decrypt-loop lands on the first index whose decrypted share matches
  `pk_set.public_key_share(i)`, which is deterministic for a fixed payload. The
  remaining open part — whether reef re-encrypts on retry — is an upstream/reef
  question, needs human input.)**

## Investigation Log

#### Is ethers `LocalWallet::sign_message` ECDSA RFC-6979 deterministic, and does it emit canonical low-S?
- **Examined:**
  - `Cargo.lock`: pinned versions are **`ethers`/`ethers-signers`/`ethers-core` = 2.0.14**
    (NOT 2.0.8 as the property body and `Cargo.toml` semver `"2.0.8"` suggest — the
    lockfile resolved up to 2.0.14), **`k256` = 0.13.4**, **`ecdsa` = 0.16.9**,
    **`elliptic-curve` = 0.13.8**.
  - `ethers-signers-2.0.14/src/wallet/mod.rs:85-93` `sign_message` → `hash_message(message)`
    (EIP-191 prefix) → `sign_hash` (:149-160) → `self.signer.sign_prehash(hash.as_ref())`.
  - `ethers-signers-2.0.14/src/wallet/private_key.rs:51` — `LocalWallet = Wallet<SigningKey>`
    where `SigningKey = k256::ecdsa::SigningKey`.
  - `ecdsa-0.16.9/src/recovery.rs:218-227` — the `PrehashSigner<(Signature, RecoveryId)>`
    impl that the wallet uses → `sign_prehash_recoverable` (:180-187) →
    `try_sign_prehashed_rfc6979::<C::Digest>(&z, &[])`.
  - `ecdsa-0.16.9/src/hazmat.rs:93-112` `try_sign_prehashed_rfc6979`: `k =
    rfc6979::generate_k::<D,_>(&self.to_repr(), &C::ORDER…, z, ad)` with **`ad = &[]`
    (empty additional data → no added entropy / no randomization)**.
  - `k256-0.13.4/src/ecdsa.rs:182-197` `SignPrimitive::try_sign_prehashed`: after
    `hazmat::sign_prehashed`, line 194 `let sig_low = sig.normalize_s().unwrap_or(sig);`
    → returns the **low-S normalized** signature. Verification (`:201-208`) rejects
    high-S (`if sig.s().is_high() { return Err }`).
- **Found:** the signing nonce `k` is RFC-6979 deterministic from `(sk, group order,
  digest)` with empty `ad`; the output `s` is normalized to the low half of the curve
  order. Both the ECDSA approval path and (separately) the BLS `sign_vem` path are
  deterministic. BLS: `blsttc` `SecretKey::sign` (`db34805/src/lib.rs:431-434`) →
  `sign_g2(hash_g2(msg))` = scalar-mult of a deterministic hash-to-curve point — **no
  nonce, deterministic by construction.**
- **Not found:** no feature flag or code path that enables randomized `k` for the
  `LocalWallet` signer (the `RandomizedPrehashSigner` impls exist in `ecdsa-0.16.9`
  but ethers does NOT call them; `sign_hash` uses the plain `PrehashSigner`).
- **Conclusion:** **RESOLVED.** For a fixed `(enclave sk, request)` the
  `enclave_signature` is **byte-identical** across runs and crash-retries, and is
  **canonical low-S** (on-chain EIP-2 verifiers will accept it). The property's
  "deterministic ⇒ idempotent" premise HOLDS; polarity does NOT flip. Assertion (2)'s
  low-S sub-check is a cheap regression sentinel rather than a live-bug detector.
  Property unchanged (still `Always identical-sig`); invariant strengthened to
  confidence HIGH on the crypto side.

  *Side note (does not change this property's polarity):* the SUT signs the
  already-`Keccak256`'d digest *via* `sign_message`, so `hash_message` re-applies the
  EIP-191 prefix and re-hashes — the on-chain preimage is therefore
  `keccak256("\x19Ethereum Signed Message:\n32" ‖ keccak256(abi.encode(...)))`. This is
  a *preimage-shape* concern for the on-chain verifier (tracked under preimage-drift in
  the binding/coordination files), not a determinism concern — it is still a pure
  function of the request.


---

> **[merged]** consolidated from discovery focus file `prop-focus-6/coordination-cross-guardian-approval-determinism.md`

# Property: identical inputs across guardians yield identical accept/reject + identical signed preimage (no version-skew divergence)

Slug: `coordination-cross-guardian-approval-determinism`
Focus: (7) Distributed Coordination — version skew / replica determinism
Instrumentation: **MISSING**

## Origin / the coordination assumption
Because the guardian set is edge-distributed (each guardian validates + signs +
submits independently; on-chain aggregates — see
`coordination-approval-preimage-matches-onchain-verifier.md`), the set only
reaches threshold if guardians AGREE on two things given the same validator
registration:
1. **the accept/reject decision** (all honest guardians that hold a valid share
   should accept; if some accept and some reject, the set may fall below
   threshold → liveness break), and
2. **the signed preimage bytes** for the part that is shared (the deposit fields
   — `bls_pub_key`, `withdrawal_credentials`, `deposit_signature`,
   `deposit_data_root`, `guardian_module_address`, `chain_id`, `validator_index`).

The acceptance decision is a pure function of S1–S6 over the request
(`src/enclave/guardian/mod.rs:260-310`): deposit-message signature valid (S1),
deposit_data_root matches (S2), bls_pub_key derivable (S3), and the guardian's
own share decrypts and matches `public_key_share(i)` (S4). No randomness, no
clock, no nonce. So in principle the decision is deterministic and identical
across guardians **iff they run the same binary and contract assumptions**.

## Where determinism can silently break across the set
1. **Version skew during a rolling upgrade (no handshake — confirmed absent).**
   There is **no version negotiation / capability check** anywhere: not between
   reef and a guardian, not between guardians (reef subagent confirmed; guardian
   client built from `guardian_url` alone). If guardians run mixed binary
   versions mid-upgrade, some may sign the 5-field and some the 7-field preimage
   (the `d119699` change), or differ on whether `verify_session` is honored —
   producing **divergent preimages / divergent decisions** with no detection.
   Result: a split set that cannot reach threshold even though every guardian is
   "honest."
2. **`fork_version` source inconsistency (W13, sut-analysis §8).** The custody
   path trusts the **payload-supplied** `fork_version`
   (`src/enclave/types.rs:146`, used in `deposit_message_root()`
   `types.rs:175-191`), while `sign-exit` and the BLS signing path use the
   process-wide `AppState.genesis_fork_version` set from env
   (`src/bin/guardian.rs:16-24, 31-33`). Two guardians started with different
   `GENESIS_FORK_VERSION` env, OR handed different payload `fork_version`, compute
   **different deposit-message roots** → S1 passes for one, fails for another →
   split decision.
3. **`threshold().to_be_bytes()` on a platform-dependent `usize`**
   (`src/enclave/shared/mod.rs:212`, in
   `build_validator_remote_attestation_payload`). Only matters on the
   `verify_session=true` reconstruction path (currently off, reef sends false),
   but if guardians run on different-width platforms it would desync the
   reconstructed attestation preimage. Low live risk today; flag.
4. **`.zip()` truncation (F2, `shared/mod.rs:200`):** if
   `guardian_eth_pub_keys.len() != bls_enc_priv_key_shares.len()`, trailing shares
   silently drop from the hashed payload — again only on the verify_session path.

## Files / functions / lines
- Decision logic (deterministic): `src/enclave/guardian/mod.rs:260-310`
  (`verify_deposit_message`, `verify_custody`); S-invariants per sut-analysis §6.
- Preimage: `src/enclave/guardian/mod.rs:322-336` (see sibling evidence file).
- fork_version inconsistency: `src/enclave/types.rs:146,175-191` (payload) vs
  `src/bin/guardian.rs:16-24,31-33` + `sign_voluntary_exit_message`
  (`mod.rs:352-370`, uses `req.fork_info`) — note exit uses caller fork_info too;
  the env genesis is used by the validator/secure-signer paths, not these
  guardian routes — VERIFY which fork actually governs custody in production.
- No version handshake: absence across `src/bin/guardian.rs` (no `/version`
  route) and reef guardian client (confirmed).

## What breaks / the risk
A liveness/agreement break: the set splits and never reaches M, so a valid
validator never provisions (or, on the exit side, can never be ejected). Unlike a
single-guardian outage (tolerated below threshold), a *determinism* bug is
**correlated**: it can split the set right at the threshold boundary in a way
that looks like "some guardians are just slow/failing."

NOTE — classic distributed properties that DO NOT apply here (single-process SUT,
on-chain coordination): consensus/leader-election/split-brain are not in this
binary. This property is specifically about **replica determinism + version
skew**, which IS real for a same-binary quorum.

## Antithesis angle
- `Always("for a fixed valid keygen payload, the accept/reject decision is
  identical across repeated/parallel evaluations")` — SUT-side `assert_always`
  comparing `verify_custody`/`verify_deposit_message` outcome to a recomputed
  reference; catches nondeterminism (it should be a pure function).
- `Always("signed preimage bytes are a pure function of (payload, chain_id,
  validator_index, module_address); no run-to-run variation")`.
- `Sometimes("two guardian instances configured with different GENESIS_FORK_VERSION
  produce different deposit_message roots for the same payload")` — a workload
  that starts two guardian configs and shows the divergence is reachable
  (demonstrates the version/config-skew hazard). If this is `Reachable`, it is a
  real coordination risk.
- No on-chain dependency for the in-process determinism assertions.

## Fault dependency
- **Node termination (restart) — DISABLED by default; useful** to model a rolling
  upgrade: restart one guardian config on a different binary/env while others stay
  up, then assert decision/preimage divergence is detected (or, ideally, that a
  version handshake rejects the mix — which today does not exist).
- Clock jitter NOT relevant (no time input to the decision).

## Open questions
- Which `fork_version` actually governs custody on-chain — payload-supplied or a
  fixed protocol value? If payload-supplied and unchecked, two guardians can be
  driven to disagree by the orchestrator (W13 is live). **Needs human input.**
- Is there a deployment policy that forbids mixed-version guardian sets (e.g.
  all-at-once upgrade)? If not, version-skew is an operational live risk and a
  `/version` endpoint + reef-side compatibility check is a cheap mitigation worth
  flagging. **Needs human input.**


---

> **[merged]** consolidated from discovery focus file `prop-focus-7/w7-4-approval-determinism-replay-anchor.md`

# W7-4 — Custody-approval signature determinism as a replay/idempotency anchor (and the property nondeterminism would break)

## Origin / intuition
The custody-approval signature is a **deterministic pure function of the request
payload** — same `(guardian_module_address, chain_id, validator_index,
bls_pub_key, withdrawal_credentials, deposit_signature, deposit_data_root)` +
same enclave sk ⇒ **byte-identical `enclave_signature`**. This is a powerful
Antithesis *replay anchor*: replay the same validate-custody request and the
approval bytes must match exactly. It is also a property that *nondeterminism
would silently break* — and there are concrete nondeterminism vectors here
(ECDSA `k`, error-path message formatting). Crucially, the determinism is *over
the wrong domain*: it includes neither `verify_session` nor the share index, so
two requests that differ ONLY in `verify_session` (true vs false) produce the
same approval bytes — meaning "the guardian verified the session" and "the
guardian skipped verification" are **cryptographically indistinguishable** in
the artifact reef forwards on-chain.

## Files / functions / line numbers
- **The signed preimage** — `src/enclave/guardian/mod.rs:319-343` `approve_custody`:
  `Keccak256(abi.encode(addr, chainId, validatorIndex, blsPubKey, withdrawalCreds,
  depositSig, depositDataRoot))`, then `wallet.sign_message(&hash)`.
- **Determinism of the ETH signature.** `LocalWallet::sign_message` (ethers)
  uses **RFC-6979 deterministic ECDSA** — so for a fixed key + fixed digest the
  signature `(r,s,v)` is deterministic. Confirms the replay-anchor is sound *in
  principle*.
- **The self-recovery check** — `mod.rs:345-347`: `sig.recover(..) ==
  wallet.address()` else `bail!`. Pure, deterministic.
- **What is NOT in the preimage:** `verify_session` (request field,
  `types.rs:89`), `session_id`, the decrypted share index `i`, the enclave's own
  pubkey. (This is SUT W1, restated here only to define the *domain* over which
  determinism holds.)
- **Same response fields are pure echoes** — `mod.rs:97-103`: `bls_pub_key`,
  `withdrawal_credentials`, `deposit_signature`, `deposit_data_root` are copied
  straight from the request, so the *entire* `ValidateCustodyResponse` is a
  deterministic function of the request.

## What breaks
- **Replay invariant (positive):** replaying an identical request must return an
  identical `enclave_signature`. If Antithesis ever observes two *different*
  approval bytes for the same request, that proves a nondeterminism leak
  (non-RFC-6979 signer swap, an `abi.encode` ordering change, a `usize`/endian
  drift, or memory corruption) — a regression that would desync the on-chain
  verifier. This is the cleanest, cheapest "same input ⇒ same output" anchor in
  the whole binary.
- **Determinism-over-wrong-domain (negative):** because `verify_session` is not
  in the preimage, the approval signed *with* attestation == the approval signed
  *without* it. Reef forwards only `enclave_signature` on-chain
  (`new_registration.rs:184` `enclave_signature: resp.enclave_signature`), so
  the chain can never tell an attested approval from an unattested one. With
  attestation OFF in production (SUT G1), *every* approval is in the unattested
  equivalence class — and turning attestation ON later changes **nothing** about
  the bytes, so there is no on-chain signal that the upgrade took effect.
- **Idempotency / replay safety:** since there is no nonce/deadline (SUT W2) and
  the output is deterministic, a captured approval is replayable forever; the
  guardian re-issues identical bytes on every retry. Good for crash-retry
  idempotency, bad for replay resistance.

## Antithesis angle
- **`Always`** (SUT-side, custody handler): *"two validate-custody calls with
  byte-identical request payloads produce byte-identical `enclave_signature`."*
  Implement by caching/hashing (request → response signature) within the
  workload and asserting equality on the second call. This is the headline
  replay anchor.
- **`Always`**: *"the approval signature recovers to the enclave wallet
  address"* — mirror of the existing `bail!` at `mod.rs:345` as a hard
  invariant (cheap, high-value, also catches signer nondeterminism).
- **`Reachable` / `Sometimes`**: *"two requests differing ONLY in
  `verify_session` yield identical `enclave_signature`"* — documents the
  determinism-over-wrong-domain collapse as a reachable state (this is the
  testable, sharper formulation of W1: not "approval omits session" but "flipping
  the security flag is a no-op on the artifact").
- Antithesis lever: it already explores request mutations and replays; this
  property turns its scheduler into a determinism fuzzer for the signer.

## Why the other six lenses miss it
- **Idempotency/Replay lens** (focus 1) reasons about *approval determinism* and
  *replay*, but the novel cut here is the **cross-domain framing**: determinism
  is a *good* anchor AND simultaneously *evidence that the security flag is
  invisible in the output* — a single property that is both a liveness/regression
  sentinel and a security-collapse witness. The pure-replay lens would assert
  "same in ⇒ same out" and stop; it would not also assert "differ only in the
  security flag ⇒ still same out" as the same coin's other face.
- **Protocol Contracts** checks the preimage byte order vs the contract; it does
  not frame the *absence* of `verify_session` from the domain as a determinism
  property reef can detect on-chain.
- **Security Boundaries** owns W1 as "approval not bound to attestation" but as a
  static binding gap, not as a runtime determinism equivalence Antithesis can
  trip.

## Open questions (and why they matter)
- Is RFC-6979 actually in force for the `ethers` `LocalWallet`/`k256` versions
  pinned here, or could a feature flag enable randomized `k`? → If randomized,
  the replay-anchor `Always` would *legitimately* fail and would instead become
  a `Sometimes(differs)` — and reef's on-chain submission of one specific
  signature is fine, but crash-retry idempotency assumptions break. *Matters
  because* it flips the assertion's polarity.
- Does any on-chain or reef logic need to distinguish attested vs unattested
  approvals? → If yes, the determinism-over-wrong-domain collapse is an exploit;
  if no, it's documentation.

## Instrumentation: MISSING
Replay comparison must be driven by the workload (cache request→signature). The
self-recovery `Always` can mirror the existing `bail!` with one SDK line.
No SDK present.

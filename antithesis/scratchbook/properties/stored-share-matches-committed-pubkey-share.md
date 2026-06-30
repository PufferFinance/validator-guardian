# Property: accepted-share-matches-committed-pubkey-share

Focus: (1) Data Integrity (the core S4 cryptographic-binding invariant)
Slug / canonical ID: `accepted-share-matches-committed-pubkey-share`

## One-sentence property
Whenever `verify_custody` **returns Ok** (and therefore the share is persisted and a
custody approval is signed), the decrypted share's `public_key_share()` must equal
`pk_set.public_key_share(i)` for the matched index `i` — a share that does not match
the committed public-key-set must never be stored or approved.

## What led to this property
- S4 (SUT §6): "A share is accepted/stored only if it decrypts under the enclave key
  **and** its public-key-share equals `pk_set.public_key_share(i)`
  (`mod.rs:291-309`). No match ⇒ no signature, no write."
- SUT §12: `test_verify_custody_with_success/_fail` exist but only assert
  `.is_ok()/.is_err()` — they never assert the *binding* between the returned share
  and the committed `pk_set` index. This is exactly the integrity guarantee that, if
  it ever silently breaks (e.g. a refactor of the decrypt loop, or a wrong-index
  match), produces a stored share that signs garbage.
- SUT §13(a): a bad custody approval → validator gets activated/funded but is
  **un-ejectable / funds stuck**. The S4 binding is what prevents approving a payload
  whose share doesn't actually belong to the committed key set.

## Code evidence (files + functions + lines)
- `src/enclave/guardian/mod.rs:281-310` `verify_custody`:
  - `:286-288` `verify_public_keys_match()?` gate (S3): `bls_pub_key` derivable from
    `bls_pub_key_set`.
  - `:291-307` decrypt loop: for each `i`, `decrypt_sk_share(i, sk)`; on success
    compare `hex(pk_set.public_key_share(i)) == hex(sk_share.public_key_share())`;
    only then `return Ok(sk_share)`.
  - `:309` else `bail!`.
- Persist + approve happen **only after** Ok: `mod.rs:79` (`sk_share = verify_custody`),
  `:82-85` write, `:88-95` `approve_custody`.
- `verify_public_keys_match`: `src/enclave/types.rs:205-209`.
- `decrypt_sk_share`: `src/enclave/types.rs:211-226` (`envelope_decrypt` then
  `SecretKeyShare::from_bytes`).

## What breaks if violated
- If the loop ever returned a share at index `i` whose `public_key_share()` ≠
  `pk_set.public_key_share(i)` (e.g. an off-by-one between the decrypt index and the
  comparison index, or a comparison weakened to always-true), the guardian would
  **persist a non-conforming share AND sign a custody approval** for a key set whose
  reconstruction it cannot actually participate in → on-chain provisioning of a
  validator the guardian set cannot later exit. This is the catastrophic
  "un-ejectable validator" outcome, and because all guardians run the same binary, a
  systematic flaw here reaches quorum (SUT §1, §13).

## Suggested assertion(s) and types
1. **`Always`** (SUT-side) at the success branch in `verify_custody`
   (`mod.rs:302-305`), immediately before `return Ok(sk_share)`: assert
   `pk_set.public_key_share(i).to_bytes() == sk_share.public_key_share().to_bytes()`.
   This *mirrors the if-condition as an invariant on the returned value*, so it can
   never spuriously fire under correct code but pins the binding against future
   regression / fault-induced memory corruption. Type rationale: every Ok return must
   satisfy it — a true safety invariant. **Instrumentation: MISSING.**
2. **`Always`** (SUT-side) immediately after `write_bls_key` at `mod.rs:85`: assert
   the filename pubkey (`sk_share.public_key_share()`) is one of the
   `pk_set.public_key_share(j)` for some `j` in range — guards the write itself
   against an index/content mismatch introduced between `verify_custody` and the
   write. **Instrumentation: MISSING.**
3. **`Sometimes`** that the decrypt loop **rejects** at least one candidate index
   before matching (i.e. `sk_share.is_err()` continue, or pk mismatch) — confirms the
   negative path is actually exercised, not just the i==my-index happy path. Message
   must be specific: "verify_custody skipped a non-matching share index". Type
   rationale: meaningful semantic state that should occur (a guardian is typically
   NOT index 0), used as an exploration anchor. **Instrumentation: MISSING.**

## Antithesis angle (faults / timing / interleaving)
- This is primarily a **correctness-under-input-variation** property: Antithesis-
  guided input generation (varying which `guardian_eth_pub_keys[i]` the enclave key
  corresponds to, varying share ordering, mismatched-length share vectors via the
  `.zip` truncation F2) probes whether the index used for decryption always equals
  the index used for the pubkey comparison.
- **Thread-pause / CPU-modulation** during the loop, plus concurrent requests sharing
  process state, can surface any non-reentrancy. (State here is local, so low, but
  the assertion is cheap insurance.)
- Combined with the `.zip` truncation in
  `build_validator_remote_attestation_payload` (SUT §7 F2), a length-mismatched
  `guardian_eth_pub_keys` vs `bls_enc_priv_key_shares` could let a share decrypt at an
  index whose pubkey-share comparison is skewed.

## Timing / config dependencies
- Pure-local, deterministic; no network/clock dependency. Reachable on every
  `validate-custody` regardless of `verify_session` (this gate runs before the
  optional session check is even relevant — actually after, but independent of it).
- Requires the enclave ETH key on disk (prior keygen) so `fetch_eth_key` succeeds and
  the loop can decrypt.

## Open questions
- **Can the matched index `i` ever differ from the guardian's true position in
  `guardian_eth_pub_keys`?** `verify_custody` trial-decrypts and matches on
  `public_key_share(i)`, but it never checks the enclave's own pubkey equals
  `guardian_eth_pub_keys[i]` (W4). *Why it matters:* if two shares could both decrypt
  under the same enclave key (they cannot in a correct DKG, but a malicious payload
  could craft `bls_enc_priv_key_shares` so an unrelated ciphertext decrypts to a valid
  `SecretKeyShare` whose `public_key_share` happens to equal `pk_set.public_key_share(i)`
  for some i), the guardian would sign for a share index it does not own. *What
  changes:* if reachable, this graduates from a data-integrity invariant into a
  W4-class authorization bug worth a dedicated `Unreachable` ("verify_custody matched
  a share at an index whose guardian_eth_pub_keys entry is not the enclave's own
  pubkey"). Needs a feasibility check on crafting such a payload.


---

> **[merged]** consolidated from discovery focus file `prop-focus-5/stored-share-matches-pubkey-share.md`

# sec-stored-share-matches-pubkey-share — A stored BLS share always matches its published public-key share (S4 boundary)

## Origin
Focus (6) Security Boundaries; SUT-analysis S4 (§6), W4; lens lead S4 ("verify this CAN'T be bypassed").

## Files / functions / lines
- `src/enclave/guardian/mod.rs:281-310` `verify_custody`:
  - `:286-288` S3: `bls_pub_key` must be derivable from `bls_pub_key_set`, else bail.
  - `:291-307` trial-decrypt loop: for each `i`, decrypt share `i` with the enclave sk;
    accept **only if** `pk_set.public_key_share(i) == sk_share.public_key_share()`
    (`:302-304`). On no match: `bail!` (`:309`) — no share returned, no write.
- `src/enclave/guardian/mod.rs:79-85` caller: the returned `sk_share` is what gets
  persisted via `write_bls_key(pubkey_share_hex, sk_share_bytes)` — i.e. the file is
  keyed by the **same** `public_key_share` that was just equality-checked.
- `src/enclave/types.rs:211-226` `decrypt_sk_share` — returns Err on bad index / bad
  ecies decrypt / bad blsttc length (the loop `continue`s past Errs).

## Precise trust assumption
This is the **key-safety boundary**: the guardian must never persist (and later
sign with) a BLS secret-key share that does not correspond to its published
public-key share at the claimed index. If a share whose `public_key_share()` differs
from `pk_set.public_key_share(i)` were ever stored under that pubkey-share filename,
later threshold reconstruction / exit signing would be corrupt or attacker-steerable.

## Adversarial sequence that would violate it
Workload supplies a `bls_enc_priv_key_shares` vector containing shares that decrypt
to a *different* secret than the published `pk_set.public_key_share(i)` for some i
(e.g. a forged share, a share for a different index, or one re-encrypted to the
guardian key). The invariant requires the equality check at `:302-304` to reject
every such case (no return, no write). A violation = the stored share's
`public_key_share()` != the `pk_set.public_key_share(i)` it was filed under.

## Real-world impact
If bypassable, an attacker could get the guardian to store/sign with a share it
shouldn't hold, undermining the threshold scheme's integrity (wrong reconstruction,
or a share an attacker can later exploit via sign-exit). This boundary is the main
thing standing between "malformed/forged share input" and "guardian holds a bad
key" — high value to prove it holds under fuzzing.

## Antithesis angle (this one is a TRUE invariant — strong asset)
- Positive guard: `Unreachable("stored a BLS share whose public_key_share does not match pk_set.public_key_share(i)")`
  placed at the persistence call (`mod.rs:82`) after recomputing
  `sk_share.public_key_share()` and comparing to the filename's pubkey-share — should
  NEVER fire.
- Coverage: `Sometimes("verify_custody rejected a non-matching share")` to confirm the
  reject branch (`:309`) is actually exercised by the adversarial workload (otherwise
  the Unreachable is vacuous).
- Workload-driven: fuzz the shares (wrong index, swapped, truncated, re-encrypted,
  random). Add thread/clock faults to probe the concurrent-write torn-file case
  (SUT §4) — but note content is deterministic so the boundary itself should hold.

## Instrumentation
**MISSING** (no SDK yet). The check itself exists in code (`:302-304`) and is a
genuine boundary; the Antithesis assertion mirrors it. Partial-present in the sense
that the equality logic is there — only the assertion is missing.

## Open questions (why they matter)
- **W4: guardian identity is inferred by trial decryption, never pinned.**
  `verify_custody` accepts "whichever share decrypts AND matches index i" but never
  checks that the enclave's own pubkey equals `guardian_eth_pub_keys[i]`. So the
  *value* check (S4) holds, but the share is not bound to *this guardian's identity*.
  Worth a companion property: `Reachable`/`Always` that the decrypting index's
  `guardian_eth_pub_keys[i]` corresponds to the enclave's own pubkey. Is the missing
  identity-pin by design? **Needs human input** — listed as the main residual risk
  even though S4 itself is sound.

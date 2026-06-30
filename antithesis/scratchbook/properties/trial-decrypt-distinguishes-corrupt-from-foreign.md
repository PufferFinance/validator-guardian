# W7-2 — Trial-decryption loop silently swallows malformed/foreign shares (a bad share never aborts, never warns)

## Origin / intuition
The wildcard prompt asked specifically: *does a malformed ciphertext in the
trial-decryption loop abort or continue?* Answer: **it continues, silently.**
`verify_custody` walks every entry in `bls_enc_priv_key_shares`, and any entry
that fails to ECIES-decrypt (wrong recipient, truncated, garbage) is skipped via
`continue` with **no log, no counter, no distinction between "not mine" and
"corrupt"**. The endpoint succeeds as long as *one* entry decrypts to a matching
share. The implicit-but-unstated guarantee is "exactly one share is mine and the
rest are simply other guardians' shares" — but the loop cannot tell a benign
foreign share from an attacker-injected malformed one, and it will accept a
payload where N-1 shares are pure garbage.

## Files / functions / line numbers
- `src/enclave/guardian/mod.rs:291-309` — the loop:
  ```
  for i in 0..keygen_payload.bls_enc_priv_key_shares.len() {
      let sk_share = keygen_payload.decrypt_sk_share(i, &guardian_enclave_sk);
      if sk_share.is_err() { continue; }          // <- malformed/foreign: silent skip
      ...
      if pk_set.public_key_share(i) == sk_share.public_key_share() { return Ok(sk_share); }
  }
  bail!("verify_custody failed to decrypt ...")
  ```
- `src/enclave/types.rs:211-226` — `decrypt_sk_share` surfaces three distinct
  failure causes as one opaque `Err`: (a) `hex::decode` failure (line 220),
  (b) `envelope_decrypt` ECIES failure (line 222), (c)
  `SecretKeyShare::from_bytes` wrong-length / invalid-Fr (line 223). The loop
  collapses all three into "skip".
- `src/crypto/eth_keys.rs:153-159` + `ecies-0.2.7/src/lib.rs:65-78` —
  `envelope_decrypt` → `ecies::decrypt` returns `Err(Error::InvalidMessage)` on
  `msg.len() < key_size`, on AES-GCM tag failure, etc. **Confirmed: it returns
  an error, never panics.** So malformed shares are well-behaved at the crypto
  layer; the *information loss* is purely in the guardian's loop.
- Decrypt is the heavy synchronous work (SUT §4): N ECIES decapsulations run on
  the async worker with no `spawn_blocking`.

## What breaks
- **Diagnostic blindness.** A custody request where this guardian's *correct*
  share has been corrupted in transit (truncated by a proxy, off-by-one in
  reef's `hex::encode`, wrong recipient ordering) is indistinguishable from a
  request that simply doesn't contain this guardian's share. Both yield the same
  generic `bail!("verify_custody failed to decrypt...")` → 500. Operators cannot
  tell "payload is malformed" from "I'm not in this set."
- **Silent partial acceptance.** A request can carry one valid share for *this*
  guardian and arbitrary junk in every other slot; the guardian happily signs
  the custody approval. Because the approval commits only to group-level deposit
  fields (W1) and not to the shares it saw, the guardian asserts nothing about
  the integrity of the *other* shares — yet reef treats a successful approval as
  evidence the whole keygen payload was well-formed.
- **DoS amplifier.** The loop runs full ECIES decapsulation for every malformed
  entry before skipping. A request with a large `bls_enc_priv_key_shares` vector
  of junk forces N expensive decaps on a blocking worker — interacts with the
  head-of-line-blocking availability fault (SUT §4) and there is no body-size /
  vector-length limit (SUT §2).

## Antithesis angle
- **`Sometimes`** (SUT-side, inside the loop on the `continue` branch):
  *"verify_custody skipped a share that failed to decrypt while still
  ultimately succeeding"* — proves the workload reaches the
  one-good-many-skipped state and that a single matching share is sufficient.
- **`Always`**: *"on a successful custody approval, the number of skipped
  (failed-decrypt) shares is strictly less than the vector length"* (i.e. at
  least one matched) — trivially true today; valuable as a regression sentinel
  if the loop is ever refactored to `?`-propagate the first error (which would
  flip behavior to abort-on-first-malformed).
- **`Reachable`**: *"verify_custody returns the generic decrypt-failure error
  when at least one share was malformed (not merely foreign)"* — confirms the
  conflation of corrupt-vs-foreign.
- **Differential angle:** assert that the count of entries that hex-decode but
  fail ECIES is `Sometimes` > 0 under fault injection (Antithesis byte-flips the
  ciphertext), and that this never changes the success/failure outcome as long
  as the matching share survives — encodes the "one good share is enough,
  garbage neighbours are tolerated" contract explicitly.

## Why the other six lenses miss it
- **Data Integrity** treats each key file independently; it never reasons about a
  *vector* of ciphertexts where some are expected to fail.
- **Failure Recovery / Resource Boundaries** would flag the missing length limit
  generically but not the semantic conflation of "corrupt" vs "not mine".
- **Protocol Contracts** checks the preimage *builders*, not the *consumer*
  loop's error handling.
- **Security Boundaries** noted W4 (identity by trial decryption) as an identity
  concern; this property is the orthogonal *robustness/observability* concern —
  the loop's inability to surface "I got handed garbage."

## Open questions (and why they matter)
- Should a custody request with ANY malformed share be rejected outright, or is
  best-effort "find my share among the noise" the intended contract? → Decides
  whether the right assertion is `Always(no malformed shares)` (reject) or
  `Sometimes(skipped > 0)` (tolerate). *Matters because* the intended trust
  model dictates whether silent skipping is a bug or a feature. **Needs human input.**
- Is `bls_enc_priv_key_shares.len()` bounded anywhere upstream (reef / contract)?
  → Determines the DoS ceiling on the blocking decrypt loop. **(partial — not
  re-investigated here; remains open from the resource-bound analysis.)**
- ~~Does `ecies::decrypt` return `Err` (not panic / not wrong plaintext) on a
  malformed or foreign ciphertext?~~ **RESOLVED — yes, always `Err`, never panic,
  never a wrong plaintext.** See Investigation Log below.

## Investigation Log

#### Does ecies-0.2.7 `decrypt` / `envelope_decrypt` fail closed (Err, not panic, not wrong plaintext) on malformed/foreign ciphertext?
- **Examined:**
  - `Cargo.lock`: `ecies = 0.2.7` (registry); also pins `libsecp256k1 = 0.7.2`.
  - SUT `src/crypto/eth_keys.rs:153-159` `envelope_decrypt` → `ecies::decrypt(&sk.serialize(),
    msg).with_context(...)?` — any `Err` is mapped through `anyhow` (no `unwrap`/`expect`).
  - SUT `src/enclave/types.rs:211-226` `decrypt_sk_share`: `hex::decode` (:220, `?` on
    bad hex), `envelope_decrypt` (:222, `?`), then
    `SecretKeyShare::from_bytes(sk_bytes[..].try_into()?)` (:223-224). The `try_into()`
    to `[u8; SK_SIZE]` is itself a **length gate** — a decrypted plaintext that is not
    exactly 32 bytes errors here before `from_bytes` is even called.
  - `ecies-0.2.7/src/lib.rs:60-78` `decrypt`: returns `Err(Error)` on (a) `parse_sk`
    failure, (b) `msg.len() < key_size` → `Err(Error::InvalidMessage)` (truncated/empty),
    (c) `parse_pk(&msg[..key_size])` failure (bad ephemeral pubkey), (d) `decapsulate`
    failure, (e) `sym_decrypt(...).ok_or(Error::InvalidMessage)` → AES-GCM tag mismatch.
  - `ecies-0.2.7/src/symmetric/mod.rs:33-47` `sym_decrypt` returns `Option` (None on
    AEAD authentication failure); the in-crate test `attempts_to_decrypt_invalid_message`
    asserts `decrypt` of garbage/empty/short input is `None`.
- **Found:** every failure branch returns a typed `Error` (no panic). Because the
  symmetric layer is **authenticated AES-GCM**, a *foreign* ciphertext (encrypted to a
  different recipient) and a *corrupt* ciphertext both fail the auth tag → `None` →
  `Err` — there is **no path that yields a wrong-but-valid plaintext**.
- **Not found:** no `unwrap`/`expect`/`panic!` in the `decrypt` path; no branch that
  returns `Ok` with attacker-influenced plaintext on a tag failure.
- **Conclusion:** **RESOLVED.** A malformed or foreign ciphertext deterministically
  produces `Err` (mapped to a generic skip in the loop), never a panic and never a
  wrong plaintext. The crypto layer is well-behaved; the property's concern is purely
  the guardian loop's *information loss* (conflating "corrupt" vs "not mine"), which
  stands unchanged. This underpins `trial-decrypt-distinguishes-corrupt-from-foreign`:
  the distinction is *lost in the loop*, not in the crypto. Property unchanged.

## Instrumentation: MISSING
The skip branch is silent (no log/metric). Counting skipped-vs-matched requires
SUT-side instrumentation inside the loop.

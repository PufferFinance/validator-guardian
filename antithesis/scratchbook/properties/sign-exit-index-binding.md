# W7-1 — Custody stores share by *intrinsic* index; sign-exit reads by *caller-chosen* index (silent identity desync)

## Origin / intuition
The guardian persists a BLS share under one key, but reads it back under a
**different, independently-chosen** key. Custody storage is keyed by the share's
*own* derived public-key-share (the index `i` in `guardian_eth_pub_keys` that
actually decrypted), while sign-exit re-derives the lookup key from a
**caller-supplied `guardian_index`** that reef hardcodes to `0`. These two
indices only coincide when this guardian happens to occupy slot 0. When they
diverge, the share is written to disk under filename A and looked up under
filename B — a "key exists but cannot be found" condition that no integrity,
concurrency, or protocol lens catches because each side is internally
self-consistent.

## Files / functions / line numbers
- **Store side** — `src/enclave/guardian/mod.rs:79-85`
  `verify_and_sign_custody_received` → `verify_custody` returns the
  `SecretKeyShare` for whichever index decrypts, then:
  ```
  write_bls_key(&hex::encode(sk_share.public_key_share().to_bytes()),  // filename = SHARE'S OWN pubkey-share
                &hex::encode(sk_share.to_bytes()))
  ```
  `sk_share.public_key_share()` (blsttc `lib.rs:520`) is intrinsic to the
  secret — it equals `pk_set.public_key_share(i)` for the matching index `i`
  (verified equal at `mod.rs:302-304`).
- **Decrypt-loop index selection** — `src/enclave/guardian/mod.rs:291-307`:
  `for i in 0..bls_enc_priv_key_shares.len()` returns the **first index whose
  ciphertext decrypts under this guardian's enclave sk AND whose
  `public_key_share(i)` matches**. So the storage filename is determined by
  *this guardian's position in the `guardian_eth_pub_keys` vector*, not by the
  caller.
- **Read side** — `src/enclave/guardian/mod.rs:355-361`
  `sign_voluntary_exit_message`:
  ```
  let pk_hex = hex::encode(req.public_key_set()?
                              .public_key_share(req.guardian_index)  // caller-chosen index
                              .to_bytes());
  let sk = fetch_bls_sk(&pk_hex)?...
  ```
  `public_key_share<T: IntoFr>(i)` (blsttc `lib.rs:717`) evaluates the
  commitment polynomial at `i+1` for **any** `i` with no bounds check — it never
  panics and always yields a syntactically valid point, so a wrong index
  produces a *valid-looking but nonexistent* filename → `read_bls_key` returns a
  plain "Unable to read secret key" error → **HTTP 500**, fails closed.
- **Cross-repo driver** — `reef/reef-guardian/src/handlers/api/webhooks/eject_validator.rs:170`
  `let guardian_index = 0;` — reef ALWAYS sends `guardian_index = 0` on sign-exit.
- **Cross-repo custody ordering** —
  `reef/.../provision_or_skip/new_registration.rs:236-260`: the guardian's
  matching custody slot is `guardian_eth_pub_keys[i]` where `i` is *this
  guardian's position in the protocol's pubkey list*, NOT necessarily 0.

## What breaks
- For every guardian whose enclave key matches share index `i != 0`, the share
  is on disk under `public_key_share(i)` but sign-exit (reef, `guardian_index=0`)
  looks up `public_key_share(0)` → file-not-found → 500. **All such guardians
  silently cannot sign exits** even though they hold a valid, verified share.
- A force-eject needs M valid sign-exit shares aggregated. If only the index-0
  guardian can ever answer, the set may **never reach threshold to eject a live
  validator** — a latent liveness failure that surfaces only the day an exit is
  needed (e.g. a slashing emergency). This is exactly the "funds-stuck /
  un-ejectable" harm in SUT §13(d), reached via a benign-looking off-by-index.
- Conversely: there is **no check that `guardian_index` corresponds to the index
  that actually decrypted at custody time**. The two endpoints share no
  consistency invariant.

## Antithesis angle
- **`AlwaysOrUnreachable`** (SUT-side, in `sign_voluntary_exit_message` after the
  share read): *"the BLS share read for sign-exit was stored under the same
  index it is now being requested under."* Encode by recording, at custody store
  time, the index `i` that decrypted (in a side file or log event) and asserting
  at sign-exit that `public_key_share(guardian_index)` equals a stored filename.
- **`Sometimes`**: *"sign-exit succeeds for a `guardian_index != 0`"* — if the
  workload exercises non-zero indices and this is never reachable, it documents
  that the binary is effectively index-0-only.
- **`Reachable`**: *"sign-exit returns 500 due to share-not-found while a share
  for that validator's pk_set exists on disk under a different index"* — the
  smoking gun for the desync.
- Antithesis lever: drive validate-custody on a guardian configured so its
  enclave key matches index `i>0`, persist the share, then drive sign-exit with
  reef's `guardian_index=0` and observe the lookup miss.

## Why the other six lenses miss it
- **Data Integrity / Idempotency** checks that a stored key round-trips under
  *the same* filename — here each side round-trips correctly under its *own*
  filename; the bug is that the two filenames are derived differently.
- **Concurrency** looks at torn reads of one path, not two endpoints disagreeing
  on which path.
- **Protocol Contracts** audits the three preimage builders, not the
  storage-key derivation.
- **Lifecycle / Distributed Coordination** reasons about M-of-N *aggregation*
  but assumes each guardian can produce its own share-signature; this property
  is precisely the case where a guardian silently can't.
- **Security Boundaries** flagged W10 (sign-exit has no auth) and W4 (identity
  by trial decryption) — but neither states the *positive* binding invariant
  that store-index must equal read-index.

## Open questions (and why they matter)
- ~~Does reef ever send a non-zero `guardian_index` on sign-exit in any code
  path, or set it per-guardian?~~ **RESOLVED (High-confidence-confirmed):** reef
  ALWAYS hardcodes `guardian_index = 0`. It is the only sign-exit call site, has
  no per-guardian index config, and never computes its own position in the
  guardian list. See Investigation Log below.
- Is `guardian_eth_pub_keys` ordering guaranteed stable/identical across keygen,
  custody, and exit for a given guardian? → If the ordering can differ between
  custody and the protocol's view, even index-0 guardians can desync.
  *(needs human input)* — partially addressed: reef builds the custody list from
  the externally-supplied webhook `guardian_eth_pubkeys` (ultimately the on-chain
  `GuardianModule.getGuardiansEnclavePubkeys()`); ordering across keygen/custody
  is plausibly the contract array order, but reef never asserts it and sign-exit
  doesn't reuse that list at all (it passes index 0 outright), so the ordering
  question is moot for the *primary* failure — the index-0 hardcode breaks the
  binding regardless of list ordering.

---

## Investigation Log

### Does reef ever send a non-zero or per-guardian `guardian_index` to `sign-exit`, or always `0`?
- **Examined:**
  - `reef/reef-guardian/src/handlers/api/webhooks/eject_validator.rs` (full file) —
    the only sign-exit caller in reef.
  - `grep -rn "guardian_index"` across all of reef (only hits: `eject_validator.rs`
    and the unrelated `db/tables/backup/vem.rs`).
  - `grep -rn "sign_exit\|sign-exit\|sign_voluntary_exit"` across reef.
  - `reef/reef-guardian/src/config/guardian.rs` + `constants/{staging,production}/guardian.json`
    (reef's guardian self-config).
  - SUT side: `src/enclave/guardian/mod.rs:60-85` (store), `:281-310` (verify_custody
    decrypt-loop), `:352-369` (sign_voluntary_exit_message read), `:583-607`
    (`test_sign_vem`, the intended contract), `src/client/tests/mod.rs:81-88,147-154`.
  - `reef/reef-guardian/src/tests/utils.rs:195-240` (4-guardian test vector).
- **Found:**
  - `eject_validator.rs:170` `let guardian_index = 0;` → `:175`
    `guardian_index: guardian_index as u64` in the `SignExitRequest`. This is the
    **only** sign-exit call site in reef.
  - **No per-guardian index config exists.** `config/guardian.rs` `GuardianConfig`
    has only `broadcast`, `enclave_url`, `enclave_public_key`, `private_key`; the
    config JSONs (`staging`/`production`) carry only `broadcast`,
    `poll_interval_seconds`, `enclave_public_key`. There is no index / slot /
    position field, and reef never `.position()`s its own `enclave_public_key`
    against any guardian list.
  - The custody side does NOT use a caller-chosen index: `verify_custody`
    (`mod.rs:291-307`) loops `for i in 0..bls_enc_priv_key_shares.len()` and stores
    the share under `sk_share.public_key_share()` (intrinsic to whichever index `i`
    decrypted under *this* guardian's enclave sk). The 4-entry test vector
    (`reef/.../tests/utils.rs`) confirms a real M-of-N (4-guardian) layout where
    `i` can be 1, 2, or 3, not just 0.
  - The SUT's own `test_sign_vem` (`mod.rs:589-598`) is the smoking gun for intent:
    it loops `guardian_index: i` for `i in 0..g_sks.len()` and pairs each with
    `verify_custody(&resp, &_g_sks[i])`. The intended contract is therefore
    *sign-exit `guardian_index` must equal the custody decrypt index `i`* — exactly
    what reef violates by always sending 0.
  - The `vem.rs` backup table's `guardian_index` column is unrelated to sign-exit
    and its `insert_record` is never called from non-test code (only
    `initialize_table` is wired in `backup_manager.rs`). Dead w.r.t. this property.
- **Not found:** Any reef code path that sends a non-zero `guardian_index`, any
  config that sets it per instance, any logic that derives reef's slot from its
  enclave pubkey.
- **Conclusion:** reef ALWAYS sends `guardian_index = 0`. Any guardian whose
  enclave key decrypts at custody index `i != 0` has its share stored on disk
  under `public_key_share(i)` but sign-exit looks it up under `public_key_share(0)`
  → file-not-found → 500, and that guardian is silently mute on exits. The SUT's
  own test proves the index was meant to vary per guardian. **Property resolves to
  High-confidence-confirmed** for the multi-guardian case; in a deployment where
  reef is configured as the slot-0 guardian it happens to work by coincidence.

## Instrumentation: MISSING
No SDK present; the store-index is not recorded anywhere, so the invariant
cannot be checked without adding a side-channel (log event or metadata file) at
custody store time. Requires SUT-side instrumentation.

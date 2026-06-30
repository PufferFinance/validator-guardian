# Property: A persisted secret key is later read back intact, or the read fails closed (never silently used corrupted)

slug: `key-write-durable-or-rejected`
focus: (3) Failure Recovery
priority: HIGH
confidence: HIGH (code path fully confirmed)

## Origin / evidence (files + functions + line numbers)

- Write path (non-atomic, never fsync'd):
  - `src/io/key_management.rs:9-14` `write_key()` = `fs::write(&file_path, sk_hex)`. This is open + `O_TRUNC` + write. **No `sync_all`/fsync of the file, no fsync of the parent dir, no temp-file + atomic `rename`, no lock, unconditional overwrite.** Confirmed by repo-wide grep: zero `sync_all|fsync|tempfile|fs::rename|NamedTempFile|persist` in `src/` (non-test).
  - `write_eth_key` (`:17-23`) and `write_bls_key` (`:26-32`) both call `write_key`.
- Read-back path:
  - `src/io/key_management.rs:49-52` `read_key()` = `fs::read` then `hex::decode(...)`. A truncated/odd-length hex file fails `hex::decode` → `Err`. A zero-length file decodes to empty `Vec<u8>`.
  - ETH read: `read_eth_key` (`:55-59`) → `crate::crypto::eth_keys::fetch_eth_key` (`src/crypto/eth_keys.rs:98-102`) → `eth_sk_from_bytes` → `EthSecretKey::parse_slice` (rejects wrong-length / invalid scalar → `Err`).
  - BLS read: `read_bls_key` (`:62-66`) → `crate::crypto::bls_keys::fetch_bls_sk` (`src/crypto/bls_keys.rs:58-65`) → `SecretKeySet::from_bytes` (rejects bad bytes → `bail!`).
- Who writes, and when, in the request flow:
  - keygen: `src/enclave/guardian/mod.rs:31` `eth_key_gen()` writes the ETH sk to disk **before** the CVM attestation await at `:56`.
  - validate-custody: `src/enclave/guardian/mod.rs:82-85` `write_bls_key(...)` persists the decrypted BLS share **before** `approve_custody` at `:88`.
- Consumers that read it back later:
  - validate-custody reads the ETH sk: `src/enclave/guardian/mod.rs:64` `fetch_eth_key(...)`.
  - sign-exit reads the BLS share: `src/enclave/guardian/mod.rs:361` `fetch_bls_sk(&pk_hex)`.

## What breaks

A VM reset / power loss / container kill between the `fs::write` returning and the bytes actually reaching durable storage (page cache not yet flushed; encrypted disk has its own write-back buffering, atakit provisions an encrypted 10GB `guardian-data` disk) can leave the key file:
- **zero-length** (truncate happened, write didn't land), or
- **truncated / torn** (partial bytes), or
- **lost entirely** (parent dir entry not durable).

The enclave ETH sk is a fresh random key per keygen (`src/crypto/eth_keys.rs:11-20`, `generate_keypair()`); there is **no backup and no cross-VM replication** (sut-analysis §2/§3). Loss is **unrecoverable** — validate-custody can never run for that pubkey again, and a stored BLS share whose file is lost means that guardian can never `sign-exit` for that validator (moves the M-of-N set toward losing eject capability — sut-analysis §13 harm (c)).

The **safety** claim we want to verify: corruption must NOT be silently used. The read-back chain (`hex::decode` → `parse_slice` / `from_bytes`) is believed to **fail closed** (returns Err → HTTP 500), so a corrupted key is rejected rather than producing a wrong signature. We want Antithesis to confirm there is no surviving-byte combination that decodes to a *different valid* key, and that the failure is a clean 500, not a panic/connection-reset.

## Antithesis angle

- Workload: drive keygen, then validate-custody, then sign-exit against the same volume, interleaved with **node-termination (kill/restart)** injected during/just-after the write window.
- After restart, re-issue the dependent op (validate-custody for the eth pk; sign-exit for the bls pk).
- Property (SUT-instrumented invariant, see below): for any key read back, the read either (a) returns the exact bytes that were intended to be written, or (b) returns an error / clean 500 — it must never return a *different valid* key, and must never panic the read path.
- Liveness companion: after `ANTITHESIS_STOP_FAULTS`, a fresh keygen + validate-custody + sign-exit cycle on a fresh pubkey must succeed (the binary recovers; old corruption doesn't wedge new work).

## Fault dependency

**REQUIRES the node-termination (kill/restart) fault — DISABLED by default. Must be explicitly enabled.** The whole property is about a crash mid-write / power-loss durability gap; with faults limited to network/clock there is no way to create the torn/zero-length file.

## Timing / config deps

- Needs the kill to land in the narrow window between `fs::write` returning and the OS flushing the page cache. Antithesis's deterministic scheduler + filesystem fault model is exactly suited to widening/hitting this window; a plain `docker kill` rarely reproduces it because the page cache usually flushes fast.
- Depends on the deployment running from CWD `/` so `./data` ⇒ the `guardian-data` volume (sut-analysis §3). If the volume is durable across restart (named docker volume), the *file-lost* variant requires the page-cache window specifically; the *new-empty-volume* variant is a separate property (see instance-affinity-restart.md).

## Instrumentation status: MISSING (SUT-side required)

This is a "dangerous + invisible" property: we can't observe "the bytes I wrote == the bytes I read" from the outside, and we must not let a corrupted key be used. Needs SUT-side Antithesis assertions:
- In `read_key` / `fetch_eth_key` / `fetch_bls_sk`: `assert_always` that *if* the file exists and is non-empty, the decoded key length is exactly the expected length (33/32 etc.), OR the function returns Err. Specifically an `assert_always_or_unreachable` that a non-empty key file never yields a successfully-parsed key of the wrong length.
- Optionally an `assert_unreachable` placed on a "used a key whose round-trip hash didn't match what keygen produced" branch — but the code keeps no such record today, so this needs a small SUT change (store a length/short-hash sidecar, or assert on parse-length only). Mark as partial: length-check assertion is cheap and present-able; full write/read identity needs a sidecar.
- The antithesis-sdk crate is not yet a dependency (existing-assertions.md) — must be added first.

## Open questions (why they matter)

- Is the `guardian-data` docker volume durable across an Antithesis node-termination, or does termination hand back a fresh non-durable FS? Decides whether this property (torn file on same volume) or instance-affinity-restart (empty volume) is the one exercised. **Matters because the two have different recovery semantics.**
- **Does any surviving-byte truncation of a secp256k1 / blsttc secret produce a
  *different but valid* key (silent wrong-key) rather than a parse error?**
  **PARTIALLY RESOLVED — depends on which key/parser:**
  - **ETH sk (`fetch_eth_key` → `EthSecretKey::parse_slice`, libsecp256k1 0.7.2):**
    length-checked (expects exactly 32 bytes) and validates the scalar is in
    `[1, n-1]` → **fails closed** on any wrong-length or out-of-range truncation. Safe.
  - **BLS share (`fetch_bls_sk` → `SecretKeySet::from_bytes` → `Poly::from_bytes`):**
    **NOT length-checked — fails OPEN for short reads.** `Poly::from_bytes` uses
    `coeff_size = bytes.len() / SK_SIZE` (integer division, SK_SIZE=32) with no
    multiple-of-32 check. The on-disk content is exactly one 32-byte coefficient
    (`SecretKeyShare::to_bytes() -> [u8; 32]`), so **any torn read yielding < 32
    decoded bytes (including 0) → `coeff_size = 0` → empty poly → `secret_key()` =
    `evaluate(0)` = `Fr::zero()` (zero key) with NO error.** This is a **silent
    wrong-key (safety) escalation** — see the detailed log in
    `torn-read-never-yields-wrong-key.md`. (Truncations that are valid hex but odd
    length fail `hex::decode`; truncations ≥ 32 bytes are impossible here because the
    full content is only 32 bytes.) See Investigation Log below.
- Is there ever a `0x`-prefixed or newline-terminated on-disk form that would make a partial read still parse? (`write_key` writes no newline, no `0x` — confirmed `:13`.)

## Investigation Log

#### Does a non-empty-but-truncated BLS key file fail closed, or also deserialize to a usable wrong key?
- **Examined:**
  - SUT write: `src/enclave/guardian/mod.rs:82-85` writes `hex(sk_share.to_bytes())`;
    `SecretKeyShare::to_bytes()` = `[u8; SK_SIZE]` (blsttc `db34805/src/lib.rs:560`,
    SK_SIZE=32) → on-disk content is exactly **64 hex chars / 32 raw bytes**.
  - SUT read: `src/crypto/bls_keys.rs:58-65` `fetch_bls_sk` →
    `io/key_management.rs:49-52 read_key` (`fs::read` → `hex::decode`) →
    `SecretKeySet::from_bytes(sk_bytes)` (bls_keys.rs:61) → blsttc.
  - blsttc `db34805/src/lib.rs:902-905` `SecretKeySet::from_bytes` → `Poly::from_bytes`.
  - blsttc `db34805/src/poly.rs:397-409` `Poly::from_bytes`: `coeff_size = bytes.len()
    / SK_SIZE`, loops `0..coeff_size` reading 32-byte chunks; **trailing partial bytes
    (len not a multiple of 32) are silently dropped — no length validation, no error.**
  - blsttc `db34805/src/poly.rs:359-370` `Poly::evaluate`: `self.coeff.last()` is
    `None` for an empty poly → **returns `Fr::zero()`**.
  - blsttc `db34805/src/lib.rs:870-874` `SecretKeySet::secret_key()` = `evaluate(0)`.
  - Per-coefficient validation: `db34805/src/convert.rs:37-44` `fr_from_bytes` rejects
    a 32-byte chunk that is not a valid field element (`Err(InvalidBytes)`).
  - Contrast path: `src/enclave/types.rs:223-224` `decrypt_sk_share` uses
    `SecretKeyShare::from_bytes(sk_bytes[..].try_into()?)` — `from_bytes` here takes a
    **fixed `[u8; 32]`** (`db34805/src/lib.rs:565`) and the `try_into()` length gate
    makes that path **fail closed** on wrong length. So the two BLS deserialization
    entry points differ: the in-memory decrypt path is length-checked; the on-disk
    `SecretKeySet::from_bytes` read path is NOT.
- **Found:**
  - **(a) Empty-input → zero-key path CONFIRMED** in the pinned blsttc source: empty →
    `coeff_size = 0` → empty `Poly` → `secret_key()` returns `Fr::zero()`, **no error.**
  - **(b) Non-empty truncated:** because the full content is only 32 bytes, a torn read
    sees 0–31 raw bytes. Any even-hex-length yielding < 32 raw bytes → `coeff_size = 0`
    → **same zero-key fail-open as empty.** Odd-hex-length → `hex::decode` `Err` (fails
    closed). A truncation landing on a clean 32-byte multiple > 0 is **not reachable**
    here (content is exactly 32 bytes) — so the only fail-open lengths are 0..<32 raw
    bytes (even hex). The dangerous outcome reduces to "any short read → zero key."
- **Not found:** no length/integrity guard in `read_key`, `fetch_bls_sk`,
  `SecretKeySet::from_bytes`, or `Poly::from_bytes`; no sidecar checksum; no
  post-read pubkey-match check in `sign-exit` (it trusts whatever deserialized).
- **Conclusion:** **RESOLVED → escalates this property's BLS branch from
  "fail-closed (liveness)" to "fails OPEN to a zero/wrong key (SAFETY)."** A short/torn
  BLS file is silently usable as the zero key. The ETH-key branch remains fail-closed
  (libsecp256k1 length+range checks). Recommended SUT-side assertion (already noted as
  MISSING): after `fetch_bls_sk`, assert the reconstructed
  `sk.public_keys().public_key_share(i)` equals the requested share pubkey, OR length
  == 32 before parse — this is the cheap guard that converts the fail-open into a clean
  rejection. Priority stays HIGH; safety classification now confirmed for the BLS path.


---

> **[merged]** consolidated from discovery focus file `prop-focus-1/persisted-bls-share-roundtrips-or-rejected.md`

# Property: persisted-bls-share-roundtrips-or-rejected

Focus: (1) Data Integrity
Slug / canonical ID: `persisted-bls-share-roundtrips-or-rejected`

## One-sentence property
A BLS secret-key share that `validate-custody` writes to disk must, when later read
back by `sign-exit`, either deserialize to the **exact same share** (its
`public_key_share(i)` still equals `pk_set.public_key_share(i)`), or be **rejected
outright** — it must never be silently used in a corrupted/truncated form to produce
an exit signature.

## What led to this property
- SUT analysis §3 ("Durability gaps"): `write_key` is `fs::write` →
  open+`O_TRUNC`+write, **never fsync'd** (neither file nor parent dir), **not
  atomic**, **no lock**, unconditional overwrite. A VM reset / power loss
  (Antithesis-injectable) can silently leave a **zero-length / truncated** file.
- SUT analysis §13(c): a lost/corrupt share "moves the set toward losing the ability
  to ever exit/eject" — i.e. a corrupted share is a real protocol harm (validator
  funds become un-exitable if ≥ N−M+1 shares are lost/corrupt).
- S4 invariant (SUT §6): a share is accepted/stored only if
  `public_key_share(i) == pk_set.public_key_share(i)` at custody time. There is **no
  equivalent check at read time** in `sign-exit`.

## Code evidence (files + functions + lines)
- Write (no integrity, non-atomic, no fsync):
  `src/io/key_management.rs:9-14` `write_key()` — `fs::write(&file_path, sk_hex)`.
  Called for BLS via `write_bls_key()` (`:26-32`), invoked at
  `src/enclave/guardian/mod.rs:82-85` inside `verify_and_sign_custody_received`.
- Read-back path (sign-exit):
  `src/enclave/guardian/mod.rs:352-370` `sign_voluntary_exit_message` →
  `crate::crypto::bls_keys::fetch_bls_sk(&pk_hex)` (`src/crypto/bls_keys.rs:58-65`) →
  `read_bls_key` (`src/io/key_management.rs:62-66`) → `read_key`
  (`:49-52`): `fs::read` then `hex::decode(...)`.
- Deserialization gate: `fetch_bls_sk` does `SecretKeySet::from_bytes(sk_bytes)`
  (`bls_keys.rs:61`); on failure it `bail!`s → 500. **Note the filename is the
  `public_key_share` hex, but the file CONTENT is the whole `SecretKeySet`
  (`sk_share.to_bytes()`) for that share** — see write at `mod.rs:83-84`
  (`pk = public_key_share`, content = `sk_share.to_bytes()`). On read, sign-exit
  looks up by `public_key_share(req.guardian_index)` (`mod.rs:356-360`).

## What breaks if violated
- **Silent corruption used to sign:** if a truncated/bit-flipped file still
  `hex::decode`s to a valid-length byte string that `SecretKeySet::from_bytes`
  accepts but is a *different* key, sign-exit produces a signature share that does
  **not** combine correctly with the other guardians' shares → the aggregated exit
  signature is invalid → the validator **cannot be exited/ejected** (funds stuck;
  SUT §13). Worse, this is invisible: the guardian returns 200.
- **Fail-closed (acceptable) case:** corruption that fails `hex::decode` or
  `from_bytes` returns 500 — registration/exit stalls but no wrong signature. The
  property's job is to *distinguish* the two: any read that produces a share whose
  `public_key_share` ≠ the requested filename's pubkey is the dangerous state.

## Suggested assertion(s) and types
1. **`AlwaysOrUnreachable`** at the sign-exit read-back point (SUT-side, in
   `sign_voluntary_exit_message` after `fetch_bls_sk`): assert that the
   reconstructed `sk.public_key_share(guardian_index).to_bytes()` hex **equals** the
   `pk_hex` used to locate the file. Today there is **no such check** — sign-exit
   trusts the file contents. Type rationale: this path is only reached when a share
   exists; "never reached" is fine, but every execution must satisfy it. A violation
   = silent corruption used to sign. **Instrumentation: MISSING.**
2. **`Always`** at the custody write point (SUT-side, immediately after
   `write_bls_key` in `verify_and_sign_custody_received`, `mod.rs:85`): read the file
   back and assert byte-identical to what was written (or that
   `from_bytes(read).public_key_share == sk_share.public_key_share()`). Catches
   torn/short writes at the moment of writing. Type rationale: this write happens on
   every successful custody, so it is an always-invariant. **Instrumentation:
   MISSING.**
3. **`Reachable`** marker on the "read produced a share, deserialization succeeded"
   branch in `fetch_bls_sk`, to confirm Antithesis actually exercises the read-back
   under fault. **Instrumentation: MISSING.**

## Antithesis angle (faults / timing / interleaving)
- **VM reset / power loss + no fsync:** Antithesis node termination (flag: DISABLED
  by default — must be enabled) or container restart between the `fs::write` and a
  later `sign-exit` can expose an unflushed/partial file. Even without termination,
  clock jitter + I/O reordering on the encrypted disk can surface short reads.
- **Concurrent torn read (no lock):** two `validate-custody` for the same share both
  `write_bls_key` (O_TRUNC) to the same path while a `sign-exit` reads — the reader
  can observe a transient empty/partial file (SUT §4). Antithesis thread-pause /
  CPU-modulation widens this window.
- **Corruption injection:** if the harness can perturb the data volume directly,
  flipping bytes in a stored share file is the cleanest trigger for assertion (1).

## Timing / config dependencies
- BLS share content is **deterministic** for a given keygen (no nonce), so any
  read-back difference is corruption, not legitimate variation — makes the invariant
  crisp.
- Requires a prior `validate-custody` (which requires a prior `keygen` on the same
  disk) before `sign-exit` can reach the read-back. Workload must chain
  keygen → validate-custody → (fault) → sign-exit.
- Node termination must be explicitly enabled in the Antithesis config to hit the
  power-loss variant.

## Open questions
- ~~**Does any truncation actually pass `SecretKeySet::from_bytes`?**~~
  **RESOLVED — yes, it is LENIENT.** `SecretKeySet::from_bytes` → `Poly::from_bytes`
  uses `coeff_size = len/32` integer division with NO length-multiple check
  (`blsttc db34805 poly.rs:397-409`), and an empty/short-read poly evaluates to the
  **zero key** with no error (`poly.rs:359-370`, `lib.rs:870-874`). It does NOT
  length-check; it only validates each whole 32-byte chunk as an in-field scalar
  (`convert.rs:37-44`). So a torn read of < 32 bytes (even hex) deserializes to a
  usable **zero key** → assertion (1) is **HIGH-yield** (lenient case confirmed). See
  the Investigation Log above and the detailed timing analysis in
  `torn-read-never-yields-wrong-key.md`. Priority confirmed HIGH.
- **Is the data volume the same one Antithesis can fault/restart?** `guardian-data`
  is an encrypted 10GB disk (atakit.json). *Why it matters:* if the harness restarts
  the container but the volume persists cleanly (journaled fs, ordered writes), the
  no-fsync window may be small. *What changes:* if the volume is ext4 with
  `data=ordered`, a rename-less `fs::write` can still leave a zero-length file after
  crash — keep High.


---

> **[merged]** consolidated from discovery focus file `prop-focus-6/lifecycle-shutdown-no-torn-key-or-orphaned-secret.md`

# Property: termination mid-request never yields a torn key file or an orphaned-but-unacknowledged secret

Slug: `lifecycle-shutdown-no-torn-key-or-orphaned-secret`
Focus: (8) Lifecycle Transitions — shutdown / crash recovery
Instrumentation: **MISSING** (post-restart filesystem assertions; SUT-side or workload-side disk inspection)

## Origin / the transition assumption
There is **NO graceful shutdown** anywhere (`grep shutdown|signal|ctrl_c|
with_graceful_shutdown` in `src/` = 0; sut-analysis §1). The server is
`axum::Server::bind(..).serve(..).await` with no signal handling
(`src/bin/guardian.rs:69-71`). The container `ENTRYPOINT` is **shell form**
(`container/Dockerfile:38` `ENTRYPOINT /usr/local/bin/${BINARY_NAME}`), so the
guardian runs as a child of `/bin/sh -c` and is **not PID 1** — on
`docker stop`/VM teardown it does not receive forwarded `SIGTERM` and is killed
abruptly after the grace period. In-flight requests are cut at an arbitrary
point.

The implicit assumption: a request is either fully applied (key written AND
caller got 200) or fully absent. Two code paths break this because writes happen
**before** the response and the write itself is **non-atomic, never fsync'd, no
rename, no lock** (`src/io/key_management.rs:9-14` `write_key` = `fs::write` =
open+O_TRUNC+write):

1. **keygen — orphaned ETH sk.** `verify_and_sign...`/`attest_new_eth_key...`
   persists the ETH sk at `src/enclave/guardian/mod.rs:31` (`eth_key_gen()`)
   BEFORE awaiting CVM-agent attestation at `:56`. Termination between `:31` and
   the 201 response leaves an ETH sk on disk the caller never learned the pubkey
   of → **orphaned secret**, no reconciliation/GC (sut-analysis §3, OQ6).
2. **validate-custody — orphaned BLS share + torn file.** Persists the decrypted
   BLS share at `src/enclave/guardian/mod.rs:82-85` (`write_bls_key`) BEFORE
   `approve_custody` signs (`:88`) and BEFORE the 200. Termination after the write
   but before the response leaves a **live BLS share on disk with no
   acknowledged approval**. Because `sign-exit` (`mod.rs:352-364`) needs only that
   share file and has **zero authorization** (sut-analysis W10), an orphaned share
   becomes a usable exit-signing capability.

Additionally, a crash *during* `fs::write` (O_TRUNC already issued, bytes not yet
flushed) leaves a **zero-length / truncated** key file. On next read,
`read_key` → `hex::decode` (`key_management.rs:49-52`) of partial bytes: realistic
outcome is fail-closed (length checks downstream reject → clean 500), but the key
is **silently and permanently corrupt with no backup** (loss of the ETH sk is
unrecoverable — each keygen is fresh random).

## Files / functions / lines
- No-shutdown: `src/bin/guardian.rs:69-71`; `container/Dockerfile:38` shell-form
  ENTRYPOINT (signal not forwarded; not PID 1).
- Non-atomic, no-fsync write: `src/io/key_management.rs:9-14` `write_key`;
  `write_eth_key` `:17-23`; `write_bls_key` `:26-32`.
- Write-before-response (keygen): `src/enclave/guardian/mod.rs:31` then `:56`.
- Write-before-response (custody): `src/enclave/guardian/mod.rs:82-85` then
  `:88-95` then handler 200 at `handlers/validate_custody.rs:10`.
- Orphaned-share → exit capability: `src/enclave/guardian/mod.rs:352-364`
  (`sign_voluntary_exit_message`, no ownership/attestation gate).
- Storage: encrypted 10GB `guardian-data` volume (`atakit.json:40-47`), mounted
  `guardian-data:/data` (`container/guardian/docker-compose.yml:15`); disk
  encryption ≠ integrity, no app-level checksum.

## What breaks / the risk
- **Orphaned secrets accumulate** across restarts/retries with no GC → growing
  set of unacknowledged ETH keys and (worse) BLS shares that are
  exit-signable without any approval record.
- **Torn/truncated key file** → either a permanently lost ETH key (cannot
  re-derive; this guardian drops out of the set → moves toward losing threshold)
  or an intermittent 500.
- Property (safety): after ANY termination, the on-disk key set is consistent —
  no truncated/zero-length key file, and no BLS share exists that the caller was
  never told about. Practically the strongest *checkable* invariant is the
  **no-torn-file** one (an existing key file is always a complete, hex-decodable,
  correctly-sized secret).

## Antithesis angle
- `Always("every key file on disk hex-decodes to a correctly-sized secret")` —
  check after restart: read each file in `eth_keys/`/`bls_keys/`, assert
  `hex::decode` succeeds and length is 33B-pk-derived / 32B-sk etc. A
  zero-length/odd-length file ⇒ violation. SUT-side (a startup self-check) or
  workload-side (inspect the mounted volume after a kill).
- `Sometimes("validate-custody persisted a BLS share but the caller received no
  200")` — if reachable under termination fault, proves the orphaned-secret
  window is real (a strong finding; argues for write-after-success ordering or a
  staging/rename+fsync).
- `AlwaysOrUnreachable("reading a key file never panics")` — guards the
  fail-closed assumption (a corrupt file should `Err`→500, never panic).
- This is exactly an `eventually_`/post-fault assertion: terminate mid-write, let
  the node restart, THEN assert disk consistency.

## Fault dependency
- **Node termination (restart) — DISABLED by default; THIS PROPERTY REQUIRES IT.**
  The whole property is about what survives an abrupt kill mid-`write_key` /
  mid-request. Without enabling restart, only the static "writes are non-atomic"
  observation stands; the actual torn-file / orphaned-secret outcomes cannot be
  produced.
- Pairs well with **disk/IO throttle** to widen the write window and make the
  torn-file interleaving more likely to be hit.

## Open questions
- Is there any reconciliation/GC for orphaned key files? **No code path found**
  (sut-analysis OQ6). If none, an explicit confirmation that this is acceptable is
  needed. **Needs human input.**
- Does the GCP CVM provide any disk-flush/crash-consistency guarantee that makes
  the missing fsync moot (e.g. ordered journaling + the secret being re-derivable
  upstream)? **Needs human input.**
- On VM teardown, what is the actual signal/kill sequence (PID-1 / init present)?
  The shell-form ENTRYPOINT suggests SIGKILL after grace — confirm with the
  deployment. **Needs human input.**

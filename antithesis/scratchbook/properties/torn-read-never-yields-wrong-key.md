# Property: Torn read of BLS share → sign-exit must never return a wrong-key (non-failing) signature

slug: `torn-read-bls-share`
focus: (2) Concurrency — TOCTOU / torn read / shared mutable state (the filesystem)

## Origin
Assigned lead "Torn read on same BLS file." Confirmed and **escalated**: the torn
read does **not** always fail closed. There is a specific interleaving where
`sign-exit` reads a freshly-truncated **empty** BLS file, deserializes it to a
**zero-coefficient polynomial**, derives the **zero secret key**, and returns
HTTP 200 with a cryptographically-invalid (wrong-key) exit signature share — no
error. The analysis (sut-analysis.md §4) assumed "fails closed but flaky"; the
empty-file sub-case is worse than that.

## Code path (files + functions + lines)

WRITE side — `validate-custody`:
- `src/enclave/guardian/handlers/validate_custody.rs:9` → `verify_and_sign_custody_received`
- `src/enclave/guardian/mod.rs:82-85` `write_bls_key(hex(sk_share.public_key_share().to_bytes()), hex(sk_share.to_bytes()))`
- `src/crypto/bls_keys.rs` → `src/io/key_management.rs:26-32` `write_bls_key` → `write_key:9-14`
  - `write_key` = `fs::write(&file_path, sk_hex)` → `open(O_WRONLY|O_CREAT|O_TRUNC)` then `write()`.
    **O_TRUNC truncates the file to zero length before the new bytes are written.**
  - No lock, no temp-file+rename, no fsync. (Confirmed: grep for
    `fsync|sync_all|rename|tempfile|Mutex|RwLock|spawn_blocking` = 0 in non-test code.)

READ side — `sign-exit`:
- `src/enclave/guardian/handlers/sign_exit.rs:9` → `sign_voluntary_exit_message`
- `src/enclave/guardian/mod.rs:356-361`:
  - `pk_hex = hex(req.public_key_set()?.public_key_share(req.guardian_index).to_bytes())`
  - `sk = crate::crypto::bls_keys::fetch_bls_sk(&pk_hex)?.secret_key()`
- `src/crypto/bls_keys.rs:58-65` `fetch_bls_sk` → `read_bls_key` (`io/key_management.rs:62-66` → `read_key:49-52` = `fs::read` then `hex::decode`) → `SecretKeySet::from_bytes`.

### Filename match proof (read reads exactly what write wrote)
- WRITE filename = `hex(sk_share.public_key_share().to_bytes())`. In `verify_custody`
  (`mod.rs:291-305`) the returned `sk_share` is the one whose
  `public_key_share()` equals `pk_set.public_key_share(i)`.
- READ filename = `hex(pk_set.public_key_share(guardian_index).to_bytes())`.
- They are byte-identical when `guardian_index == i` — i.e. the index this
  guardian holds. So sign-exit reads precisely the file validate-custody writes.
  No path mismatch; the collision is real.

## What breaks — three observable outcomes of the read, depending on timing

Deserialization chain on the read: `fs::read` → `hex::decode` →
`SecretKeySet::from_bytes` → `Poly::from_bytes` → `secret_key()=poly.evaluate(0)`.
blsttc source: `~/.cargo/git/checkouts/blsttc-.../db34805/src/{lib.rs,poly.rs,convert.rs}`.

1. **Empty file observed (WRONG-KEY, fails OPEN — the dangerous case).**
   `fs::read` returns `[]` (file was just O_TRUNC'd, write not yet done).
   - `hex::decode([])` = `Ok([])` (empty is valid hex).
   - `Poly::from_bytes(vec![])` (poly.rs:397-409): `coeff_size = 0/32 = 0` → `Poly{coeff:[]}`. **No error.**
   - `SecretKeySet::secret_key()` (lib.rs:872) → `poly.evaluate(0)` (poly.rs:359-366):
     `self.coeff.last()` is `None` → **returns `Fr::zero()`**.
   - sign-exit then signs the VEM root with the **zero BLS key** (`sign_vem`,
     `mod.rs:372-394`) and returns **HTTP 200** with a deterministic but invalid
     signature share. **No fail-closed.** The wrong share, if it reached
     aggregation across guardians, is a junk/invalid threshold signature.

2. **Partial / mid-write content observed (USUALLY fails closed, possibly torn-valid).**
   - Odd number of hex chars or a non-hex byte → `hex::decode` errors → 500.
   - Even-length but not a multiple of 64 hex chars (32 bytes): `Poly::from_bytes`
     uses `coeff_size = len/32` (**integer division**, poly.rs:399) and **silently
     drops the trailing partial coefficient** — no length-multiple check. The
     surviving whole-coefficient prefix is then validated per-coeff by
     `fr_from_bytes` (convert.rs:37-43, rejects out-of-field scalars). Because the
     content written is deterministic (same share each time), a prefix of the
     correct bytes that happens to be a clean 32-byte multiple **could**
     deserialize to a *lower-degree but in-field* polynomial → a different,
     wrong `secret_key()` with **no error** (fails open) — same class of bug as (1),
     just needs a 32-byte-aligned torn length. Most other lengths fail closed (500).

3. **Fully-written file observed (correct).** Normal success.

Net: under the write→read race there exists a non-empty set of interleavings
(empty file; 32-byte-aligned partial) where sign-exit **returns 200 with a wrong
signature** rather than erroring. That is a *safety* violation (silent wrong
crypto output), not merely a liveness/flakiness issue.

## Exact interleaving / timing window
Two requests on the same tokio multi-thread runtime (`#[tokio::main]`, workers =
num CPUs), both doing **synchronous blocking fs** directly on worker threads (no
`spawn_blocking`):

- T1 `validate-custody` (re-registration / custody refresh for share index i):
  reaches `write_bls_key` → `fs::write` → kernel `open(...,O_TRUNC)` truncates file
  to 0 bytes  ──┐ window opens
- T2 `sign-exit` (guardian_index = i): `fetch_bls_sk` → `fs::read` observes the
  0-byte (or partially-written) file  ◄── window
- T1 `write()` completes  ──┘ window closes

Window = the gap between O_TRUNC and the write() completing. On a slow encrypted
disk (GCP CVM `guardian-data`), under node-throttle or io-latency faults, this
window is widened from microseconds to easily-hit. Antithesis thread-pausing /
CPU-modulation can pause T1 between the truncate and the write to force the
observation deterministically.

Realism: same guardian process serves both endpoints (`bin/guardian.rs:52-64`).
reef calls `sign-exit` per ejection and `validate-custody` per registration as
independent webhook flows (`reef-guardian/.../eject_validator.rs:193`,
`.../new_registration.rs`). A validator being re-registered (new custody write
for the same BLS group) while an ejection is processed hits the same file. Not
purely adversarial.

## Antithesis angle
- Faults: **thread-pausing + CPU-modulation** to interleave the truncate/write
  against the read; **io-latency / node-throttle** to widen the window.
- Workload: concurrently drive `validate-custody` (same share, repeated) and
  `sign-exit` (same `guardian_index`) against the same volume.
- Assertion (SUT-side, requires instrumentation): in `sign_voluntary_exit_message`
  immediately after `fetch_bls_sk(...).secret_key()`, before signing —
  `Always` that the loaded `SecretKeySet` is non-degenerate: e.g. assert the
  derived public key share equals `req.public_key_set()?.public_key_share(i)`
  (recompute and compare). If they differ (zero key / truncated poly), the
  assertion catches the wrong-key read. Equivalent framing:
  `Unreachable` that sign-exit returns a signature whose share pubkey ≠ the
  requested share's pubkey.

## Instrumentation status
**MISSING.** No Antithesis SDK in the crate. Also no in-code guard: sign-exit
does **not** validate that the loaded key matches the requested public-key share
(it trusts whatever deserialized). The simplest fix *and* the natural assertion
anchor is the same check: recompute `sk.public_keys().public_key_share(i)` and
compare to the requested share pubkey before signing.

## Open questions (why they matter)
- ~~Can a torn read ever land on a 32-byte-aligned partial length in practice~~
  **RESOLVED — the relevant on-disk content is exactly ONE 32-byte coefficient, so
  the "aligned-partial > 0" sub-case (case 2) is NOT reachable; the only fail-open
  lengths are 0..<32 raw bytes, which all collapse to the zero-key (case 1).** See
  Investigation Log. The property needs only **"short-read → zero key"** coverage;
  the multi-coefficient aligned-partial scenario does not arise for a stored share.
- Does any aggregation/verification step downstream (reef, on-chain
  `GuardianModule`) reject a zero-key share, turning the safety bug back into a
  liveness bug? Even if so, the guardian itself emitting a 200 + wrong signature
  is the invariant we can assert in-process. *Matters for severity framing, not
  for whether the guardian-local property holds.* **(partial — guardian-local
  property holds regardless; downstream rejection is a cross-repo/reef question,
  needs human input.)**
- Is re-running `validate-custody` for an already-stored share a real production
  flow (retry / re-registration), or is each share written exactly once? If
  exactly once, the racing writer is the *first* custody write racing the *first*
  sign-exit; still reachable but narrower. *Matters for how the workload sequences
  requests.* **(needs human/reef input — see also the duplicate-retry note in
  `approval-deterministic-and-idempotent.md`.)**

## Investigation Log

#### (a) Confirm the empty-input → zero-key path in the pinned blsttc source. (b) Does a non-empty-but-truncated file fail closed or yield a usable wrong key?
- **Examined (pinned blsttc git `PufferFinance/blsttc` @ `db34805`,
  `~/.cargo/git/checkouts/blsttc-5408fa24f1e274fe/db34805/src/`):**
  - `lib.rs:560` `SecretKeyShare::to_bytes() -> [u8; SK_SIZE]` (SK_SIZE=32, `lib.rs:50`):
    the written file content is exactly **32 raw bytes / 64 hex chars** (one coefficient).
  - `lib.rs:902-905` `SecretKeySet::from_bytes(Vec<u8>)` → `Poly::from_bytes`.
  - `poly.rs:397-409` `Poly::from_bytes`: `coeff_size = bytes.len() / SK_SIZE`
    (integer division), reads `coeff_size` 32-byte chunks; **no check that
    `bytes.len()` is a multiple of 32 — trailing partial bytes are silently dropped.**
  - `poly.rs:359-370` `Poly::evaluate(i)`: `match self.coeff.last() { None => return
    Fr::zero(), … }` — an empty coeff vec yields **`Fr::zero()`**.
  - `lib.rs:870-874` `SecretKeySet::secret_key()` = `SecretKey::from_mut(&mut
    poly.evaluate(0))`.
  - `convert.rs:37-44` `fr_from_bytes`: each whole 32-byte chunk is validated as an
    in-field scalar (`Fr::from_bytes_be`; `Err(InvalidBytes)` if out of field).
  - SUT read chain: `src/crypto/bls_keys.rs:58-65` `fetch_bls_sk` →
    `io/key_management.rs:49-52 read_key` (`fs::read` → `hex::decode`).
- **Found:**
  - **(a) CONFIRMED:** empty file → `hex::decode([]) = Ok([])` → `Poly::from_bytes(vec![])`
    → `coeff_size = 0` → `Poly{coeff:[]}` (no error) → `secret_key() = evaluate(0) =
    Fr::zero()`. `sign-exit` then signs with the zero key and returns HTTP 200.
  - **(b) Non-empty truncated:** because full content is only 32 bytes, a torn read can
    only observe 0–31 raw bytes. (i) odd-length hex → `hex::decode` `Err` → 500
    (fail-closed). (ii) even-length hex decoding to 1..31 bytes → `coeff_size =
    floor(<32 / 32) = 0` → **empty poly → same zero-key fail-open as (a).** (iii) a
    clean 32-byte-aligned partial of length 32, 64, … is **NOT reachable** for a stored
    share (the whole payload is 32 bytes; there is no 64th byte to truncate to). The
    multi-coefficient "aligned-partial → lower-degree in-field poly → different valid
    key" scenario the property body hypothesised (case 2) therefore **does not occur
    for a persisted single-share file**; the realised danger is solely the zero-key.
- **Not found:** any length guard, multiple-of-32 check, or post-read pubkey-match
  validation in the SUT or blsttc on the `SecretKeySet::from_bytes` path. (The *other*
  blsttc entry point, `SecretKeyShare::from_bytes([u8; 32])` used by `decrypt_sk_share`
  in `types.rs:223`, IS fixed-length and fails closed — but that is the in-memory
  decrypt path, not this on-disk read path.)
- **Conclusion:** **RESOLVED and CONFIRMED.** The torn-read → wrong-key (zero-key)
  fail-open is real and reduces to a single, robust trigger: any short read (0..<32
  bytes, valid even-length hex) of the BLS share file deserializes to the **zero key**
  with no error and HTTP 200. This is a **safety** violation (silent wrong crypto
  output). The "aligned-partial > 0" sub-case is ruled out for persisted shares, so
  the workload/assertion only needs to cover the short-read/empty case (which needs
  only the O_TRUNC-before-write window — robust regardless of `write(2)` atomicity).
  The natural SUT-side guard/assertion is unchanged: recompute
  `sk.public_keys().public_key_share(i)` and compare to the requested share pubkey
  before signing. Property unchanged in substance; open questions on reachability
  resolved (now crisper: "short read" not "aligned partial").


---

> **[merged]** consolidated from discovery focus file `prop-focus-2/concurrent-custody-write-durability.md`

# Property: concurrent unlocked overwrites of the same BLS file leave a valid (or no) key, never a corrupt one

slug: `concurrent-custody-write-durability`
focus: (2) Concurrency — concurrent writers + crash atomicity of shared FS state

## Origin
Derived from the lead "two `validate-custody` for the same share both
`write_bls_key` to the same path" combined with the "no atomic write/lock/fsync"
durability finding (sut-analysis §3, §4). This is the *writer×writer* and
*writer×crash* sibling of the torn-read property (which is writer×reader).

## Code path
- Two concurrent `validate-custody` for the same BLS share (same registration
  retried, or two reef flows for the same validator) →
  `verify_and_sign_custody_received` (`mod.rs:60`) → both reach
  `write_bls_key(samepath, samebytes)` (`mod.rs:82-85`) →
  `io/key_management.rs:write_key:9-14` = `fs::write` (O_TRUNC, no lock).
- Content is **deterministic** for a given share (same decrypted `sk_share`
  bytes), so two writers write *identical* bytes. The danger is not divergent
  content but the **interleaving of two open(O_TRUNC)+write pairs** and a
  **crash/VM-reset between truncate and write** (Antithesis node-termination —
  DISABLED by default, must be enabled; or power-loss/clock fault).

## What breaks
- Writer×writer: two `fs::write` of identical bytes to the same path. Worst
  realistic outcome is interleaved truncate/writes that momentarily shorten the
  file (this is exactly the window the torn-read property exploits on the read
  side). Because bytes are identical, the *final* on-disk content after both
  complete is correct. So **no wrong-key persists** from writer×writer alone —
  this is the bounded-good case worth asserting (rules out a scarier hypothesis).
- Writer×crash: O_TRUNC executes, then the VM resets before `write()` lands and
  before any fsync (there is **no fsync/sync_all** anywhere — confirmed grep=0,
  and no temp-file+rename). On restart the BLS file is **zero-length or absent**.
  A subsequent `sign-exit` then hits the empty-file → zero-key path (see
  `torn-read-bls-share.md` case 1) or a read error (500). The share is silently
  lost with no backup/reconciliation (F9 orphan/no-GC).

## Exact interleaving / timing window
- W1 and W2 both enter `write_key`; interleave their `open(O_TRUNC)`/`write`.
  Window where the file is < full length is the same one the reader race uses.
- Crash variant: pause/terminate the node in the gap between O_TRUNC and the
  fsync-less write completing (and OS page-cache flush). Without fsync, even a
  "completed" `fs::write` is not durably on disk at VM-reset time.

## Antithesis angle
- Faults: thread-pausing/CPU-modulation (interleave the two writers);
  **node-termination (must be explicitly enabled — flag)** + clock jitter to model
  VM reset between truncate and durable write.
- Assertions (SUT-side instrumentation):
  - `Always` (invariant): after `write_bls_key` returns Ok, the file
    re-reads/deserializes to the **same** `SecretKeySet` that was intended
    (round-trip check) — catches a write that "succeeded" but isn't durable/intact.
  - `AlwaysOrUnreachable`: on any successful `sign-exit`, the loaded key's public
    share equals the requested share (shared with the torn-read property; this is
    the consolidated safety invariant covering both writer×writer and writer×crash
    consequences).

## Instrumentation status
**MISSING.** No fsync/rename/lock today; no SDK. This property mostly overlaps the
torn-read safety invariant on the read side; its distinct contribution is the
**crash-durability** angle (key loss after VM reset) which is a *liveness/data-loss*
property rather than a wrong-key safety property.

## Open questions (why they matter)
- Is node-termination going to be enabled for this harness? It is DISABLED by
  default. Without it, the writer×crash sub-case can't be exercised and this
  collapses into "writer×writer is benign" (a weaker, mostly-confirmatory
  property). *Matters for whether this property earns its own test budget or folds
  into the torn-read one.*
- Is there any retry that re-issues `validate-custody` for an already-written
  share? If writes are strictly once-per-share, writer×writer needs two
  *distinct* concurrent registrations colliding on the same share index, which is
  rarer. *Matters for realism of the writer×writer interleaving.*

## Honest assessment
This is the weakest of the three concurrency properties: the deterministic-content
fact means writer×writer cannot by itself produce a wrong key, and the dangerous
consequences (empty/lost file → wrong-key on read) are already captured by
`torn-read-bls-share`. Keep it only if crash-durability (node-termination/clock)
is in scope; otherwise treat as a sub-case of the torn-read property.


---

> **[merged]** consolidated from discovery focus file `prop-focus-3/concurrent-torn-read-bls-share.md`

# Property: a concurrent write+read on the same BLS share file never yields a wrong key — at worst a fail-closed transient error

slug: `concurrent-torn-read-bls-share`
focus: (5) Resource Boundaries (lock-free FS) ∩ (3) Failure Recovery
priority: MEDIUM
confidence: HIGH (lock absence confirmed)

## Origin / evidence

- **No locking around the filesystem** anywhere in production code (only the test mock has a `Mutex`; sut-analysis §4). `write_key` (`src/io/key_management.rs:9-14`) opens with `O_TRUNC` and writes, with no advisory lock; `read_key` (`:49-52`) does a plain `fs::read`.
- Same-path contention is reachable:
  - Two `validate-custody` for the **same** share both call `write_bls_key` to the same path (filename = the share's public-key-share hex), `mod.rs:82-85`. The content is deterministic (same share ⇒ same bytes), so there is no *wrong value* — but the O_TRUNC + write is not atomic.
  - A concurrent `sign-exit` reader (`mod.rs:361` `fetch_bls_sk` → `read_bls_key`) can observe the file mid-write: **zero-length** (after truncate, before write) or **partially written**.
- Read-back fails closed: a torn read → `hex::decode` error or `SecretKeySet::from_bytes` error (`bls_keys.rs:61-64`) → `?` → 500 (`sign_exit.rs:12-19`). No path uses a partially-read key to sign.

## What breaks

- **Intermittent 500s** on sign-exit (and theoretically on validate-custody's own eth-key read, though that path doesn't rewrite the eth key) when a write to the same file is in flight — flaky liveness, not a safety break. Because content is deterministic, there is no risk of signing with a *different valid* key from this race specifically.
- The safety claim to verify: under heavy concurrent write+read on the same path, sign-exit **never** produces a signature from a torn/empty buffer — it either signs with the correct full key or returns an error. (Distinct from the crash-durability case in key-write-durable-or-rejected.md, which is about post-restart corruption; here it's a live in-flight torn read.)

## Antithesis angle

- Workload: repeatedly issue validate-custody (same share) concurrently with sign-exit (same share pk), maximizing interleaving; Antithesis's deterministic scheduler explores the truncate-then-write vs read window.
- Properties:
  - `Always("sign-exit returns either a valid signature or an HTTP error — never a signature over a wrong/short key")` — SUT-instrumented: assert the BLS key bytes read are exactly BLS_PRIV_KEY size before signing (or the SecretKeySet parse succeeded), else error.
  - `Reachable("sign-exit observed an empty/torn BLS share file during a concurrent write")` — proves the race window is real (the bug evidence; argues for a lock or atomic rename).
  - `AlwaysOrUnreachable("torn read of BLS share is rejected, not signed")`.

## Fault dependency

- Does **NOT** require node-termination. Pure concurrency in the workload; optionally amplified by disk-latency / thread-pause faults to widen the truncate→write window. No special fault needed for reachability.

## Timing / config deps

- The torn-read window is tiny (truncate→write of ~64 hex bytes); widening it needs disk-latency / CPU throttle faults and high concurrency. Antithesis scheduling is the right tool; a normal stress test rarely hits it.

## Instrumentation status: MISSING (SUT-side)

- "Never sign over a torn key" needs a length/parse guard assertion right before `sign_vem` consumes the share (`mod.rs:361-364`) — `assert_always` that the fetched `SecretKeySet` parsed and is the expected size. antithesis-sdk not yet a dependency.
- The transient-500 reachability is workload-observable (intermittent 500 under concurrency) — partial.

## Open questions (why they matter)

- Can two *different* shares ever map to the **same** filename (pk-share hex collision across validators)? If so, a concurrent write could swap content (wrong-key risk escalation). Believed no (pk-share is unique per (set, index)), but worth confirming. **Matters: would turn flaky-500 into a real wrong-key safety bug.**
- Is sign-exit ever called concurrently with validate-custody for the same pk in the real reef flow, or are they strictly sequential per validator? Cross-repo. **Matters: determines whether the race is reachable in production or only adversarially.**

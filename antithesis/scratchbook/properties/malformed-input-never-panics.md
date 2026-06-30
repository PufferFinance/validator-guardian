# malformed-input-never-panics

Synthesized during catalog assembly from failure-mode findings F7/F8/F11 (see
`sut-analysis.md` §7) plus the "no `CatchPanicLayer`" cross-cutting fact. The
lifecycle agent ([[missing-state-request-fails-clean]]) touched the panic angle
but framed it around missing prerequisite state; this property isolates the
**malformed-input → panic** vector, which is distinct (it fires even when all
prerequisite state exists).

## What led to this property

The guardian router has **no `tower` middleware at all** — specifically no
`CatchPanicLayer` (`src/bin/guardian.rs:35-65`). A panic inside any axum handler
therefore **drops the TCP connection with no HTTP response** (the worker task
unwinds; the process survives; the caller sees a connection reset, not a 500).
Several panics are reachable directly from request-controlled fields:

- **F8 (most severe — reachable on *every* `validate-custody`):**
  `BlsKeygenPayload::signature()` does
  `sig_bytes.copy_from_slice(&hex::decode(&sanitized)?)` into a fixed `[u8;96]`
  with **no length check** (`src/enclave/types.rs:167-173`). A `signature` field
  that hex-decodes to ≠ 96 bytes panics. This runs in `verify_deposit_message`
  (`mod.rs:265`) on every custody call, **regardless of `verify_session`**.
- **F7:** `verify_session_evidence` does
  `dd_root.copy_from_slice(&hex::decode(&keygen_payload.deposit_data_root)?)`
  into `[u8;32]` with no length check (`mod.rs:135-136`). Panics on a
  `deposit_data_root` ≠ 32 bytes. (Only on the `verify_session=true` path.)
- **F11:** `ListKeysResponse::new` indexes `pk[0..2]` (`src/enclave/types.rs:50`)
  — panics on a short/odd filename present in the eth-keys dir, reachable via
  `GET /eth/v1/keygen`.
- Related: `withdrawal_credentials()` and `public_key_set()` use checked
  conversions and `bail!` (no panic) — contrast confirms the inconsistency is
  per-field, not systematic.

## What goes wrong if violated

- The caller (reef) sees a connection reset instead of a structured error,
  complicating its error handling and retry logic; combined with no client
  timeout (see [[dependency-hang-makes-progress]]) this can stall provisioning.
- If `/guardian/v1/validate-custody` is reachable by an **untrusted** client
  (open question Q4), F8 is a trivial **remote denial-of-service** — one tiny
  malformed request per worker thread, and there is no panic recovery, no body
  limit, and the heavy work is on the async workers (see
  [[upcheck-live-under-load]]).

## Antithesis angle

Pure **input-fuzzing + reachability** property, optionally amplified by
concurrency. The workload sends `validate-custody` / keygen / list requests with
hex fields of wrong lengths (signature ≠ 96 B, deposit_data_root ≠ 32 B), odd-
length hex, non-hex, empty, and oversized values. Antithesis explores the input
space and the thread interleavings; thread-pause/throttle widen any window where
a panic mid-handler interacts with a concurrent request on shared FS state.

## Suggested instrumentation

- `Unreachable` ("guardian handler panicked on malformed input"): the cleanest
  encoding is to add a `CatchPanicLayer` (or per-handler catch) that, on catching
  a panic, fires an Antithesis `assert_unreachable!` and returns a 400. **MISSING**
  — requires (a) adding the `antithesis-sdk` crate and (b) adding the catch layer,
  which the guardian does not currently have.
- Defensively, the underlying fix is to replace the `copy_from_slice` calls with
  length-checked conversions that `bail!` like the sibling methods — turning the
  panics into clean 400/500s. The property would then assert
  `Always("malformed-length field yields a structured error, never a panic")`.
- `Sometimes` ("a malformed-length signature was rejected with a 4xx") to confirm
  the negative path is exercised once a fix lands. **MISSING.**

## Open Questions

- Is `/guardian/v1/validate-custody` reachable by untrusted clients in the TDX
  network topology, or only by reef over a private link? *(why it matters: decides
  whether F8 is a remote DoS or a robustness/defense-in-depth issue — sets
  priority from High to Medium.)* Cross-refs the catalog-wide Q4 in
  `sut-analysis.md`. **(needs human input — network topology, not in code.)**
- ~~Are there other unchecked `copy_from_slice` / slice-index sites reachable from
  guardian routes beyond F7/F8/F11?~~ **RESOLVED** — full grep of the request path
  done; the complete inventory is exactly **F7, F8, F11** (plus a fourth panic on
  the `verify_session=true` path that is a *prerequisite* of F7: a non-hex
  `deposit_data_root` would be caught by `?` first, but a valid-hex wrong-length
  `deposit_data_root` panics at the same `copy_from_slice`). No additional panic
  sites exist on the four guardian routes. See Investigation Log below.

## Investigation Log

### Complete inventory of panic sites reachable from the four guardian routes

**Examined:** `bin/guardian.rs` (router), all three guardian handlers
(`validate_custody.rs`, `sign_exit.rs`, `attest_fresh_eth_key_with_blockhash.rs`),
the shared `list_eth_keys`/`health` handlers, `enclave/types.rs`,
`enclave/guardian/mod.rs`, `enclave/shared/mod.rs`, `crypto/eth_keys.rs`,
`crypto/bls_keys.rs`, `io/key_management.rs`, plus the `strip_0x_prefix!` macro
(`lib.rs:14-18`) and the `FixedVector` git dep (`From<Vec>` impl). Ran greps for
`copy_from_slice`/`clone_from_slice`, `.unwrap()`/`.expect()`/`panic!`/
`unreachable!`/`todo!`, slice indexing (`[0..`, `[..`, `[i]`), `try_into`, and
`as` casts across the request path (test modules excluded).

**Found — the complete reachable-panic inventory (3 distinct sites):**

| # | Site (file:line) | Construct | Route(s) | Triggering input |
|---|---|---|---|---|
| F8 | `enclave/types.rs:171` `sig_bytes.copy_from_slice(&hex::decode(&sanitized)?)` into `[u8;96]` | unchecked `copy_from_slice` (len ≠ 96) | `validate-custody` (always: `verify_deposit_message`→`signature()` at `mod.rs:265`; also `approve_custody` `mod.rs:334`) | `signature` field whose hex decodes to ≠ 96 bytes. Non-hex is caught by `?`; valid-hex wrong-length panics. |
| F7 | `enclave/guardian/mod.rs:136` `dd_root.copy_from_slice(&hex::decode(&deposit_data_root)?)` into `[u8;32]` | unchecked `copy_from_slice` (len ≠ 32) | `validate-custody` **only when `verify_session=true`** (`verify_session_evidence`, runs at `mod.rs:72` *before* F8) | `deposit_data_root` whose hex decodes to ≠ 32 bytes. |
| F11 | `enclave/types.rs:50` `pk[0..2].into()` | string slice `[0..2]` on possibly-short string | `GET /eth/v1/keygen` (`ListKeysResponse::new`) | a filename in the eth-keys dir shorter than 2 bytes / not on a char boundary (FS-controlled, not request-body). |

**Ordering note (refines F7 vs F8 on the `verify_session=true` path):** in
`verify_and_sign_custody_received` the call order is `verify_session_evidence`
(`mod.rs:72`) → `verify_deposit_message` (`mod.rs:76`). So with
`verify_session=true`, F7's `dd_root.copy_from_slice` (and the structural
`bail!` checks for empty session fields) are hit **before** F8. With
`verify_session=false` (the always-reachable path), F8 is the first panic.

**Not found (confirmed *safe* — these `bail!`/`?` on bad length, no panic):**
- `BlsKeygenPayload::withdrawal_credentials()` (`types.rs:155-165`) — explicit
  `if wc_bytes.len() != 32 { bail! }` guard *before* `copy_from_slice`. Safe.
- `decrypt_sk_share` (`types.rs:211-226`) — `.get(idx)` (no index panic),
  `try_into()?` (fallible, no panic) on the decrypted bytes. Safe.
- `eth_pk_from_hex` / `eth_pk_from_hex_uncompressed` (`crypto/eth_keys.rs:38-79`) —
  length-checked `if pk_bytes.len() != … { bail! }` before `clone_from_slice`. Safe.
- `&hex::decode(&keygen_payload.signature)?.into()` → `BLSSignature` (Bytes96) at
  `mod.rs:139`, and the `fork_version: Version` (Bytes4) request field: these go
  through `ssz_types::FixedVector`'s `From<Vec>` impl
  (`fixed_vector.rs:102-105`), which `resize_with`s (truncates/zero-pads) — **does
  not panic** on wrong length. Confirmed against locked git rev.
- `session_id` / `workload_id` length checks (`mod.rs:155-160, 205-210`) — `bail!`
  guarded before `B256::from_slice`. Safe.
- `strip_0x_prefix!` (`lib.rs:16`) is `strip_prefix("0x").unwrap_or(&hex)` — the
  `unwrap_or` makes it total; no panic.
- All `.parse()`/`.unwrap()`/`.expect()` hits in the request path are either inside
  `#[cfg(test)]` modules or are `?`-propagated `.parse()?` (e.g.
  `guardian_module_address.parse()?` `mod.rs:323`, `registry_address …
  .parse().map_err(…)?` `mod.rs:172-177`). The only `.expect()`s
  (`guardian.rs:14,23`) are **startup** (port / genesis_fork_version), not
  request-reachable.

**Conclusion:** The reachable-panic inventory across the four routes is **exactly
F7, F8, F11** — no additional unchecked `copy_from_slice`/slice/`try_into`/cast
panic sites exist on the request-handling path. F8 is the only one reachable on
**every** `validate-custody` call regardless of `verify_session`. Open question
"any other unchecked sites?" is **resolved**.

### No CatchPanicLayer / TimeoutLayer / body-size limit on the router

**Examined:** `bin/guardian.rs:35-65` (the `axum::Router` construction) and
`Cargo.toml`. Grepped the whole repo for `tower`, `tower_http`, `CatchPanicLayer`,
`TimeoutLayer`, `ConcurrencyLimit`, `DefaultBodyLimit`, `RequestBodyLimit`.

**Found:** The router has **no `.layer(...)` calls at all** — four `.route()`s and
one `.with_state()`, nothing else. `Cargo.toml` depends on `axum = "0.6.20"` and
`tokio = { "1", features=["full"] }`; there is **no `tower`/`tower-http`
dependency**. Zero matches for any catch/timeout/concurrency/body-limit layer.

**Not found:** any panic-recovery, request timeout, concurrency cap, or body-size
cap anywhere in the binary or its deps wiring.

**Conclusion (axum 0.6 panic semantics):** In axum 0.6 each connection is served by
a hyper task that polls the handler future; an un-caught panic in a handler unwinds
that task. With no `CatchPanicLayer`, hyper observes the service future panicking
and **aborts the connection without sending any HTTP response** (the client sees a
connection reset / incomplete response, **not** a 500). The process and other
worker threads survive (panic = thread-local unwind, not `abort`), so this is a
per-request connection drop rather than a crash. This confirms the property's
core premise: malformed-length input on `validate-custody` (F8) drops the TCP
connection with no structured error. **Confirmed; no property change.**

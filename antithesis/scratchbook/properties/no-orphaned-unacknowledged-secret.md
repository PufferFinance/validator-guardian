# Property: Every persisted secret eventually corresponds to a returned approval/response, or is reconcilable (no silent orphan)

slug: `orphaned-secret-write-before-success`
focus: (3) Failure Recovery — write-before-success ordering (F9)
priority: HIGH
confidence: HIGH (ordering confirmed in code)

## Origin / evidence

- **keygen orphan window:** `src/enclave/guardian/mod.rs`
  - `:31` `let pk = crate::crypto::eth_keys::eth_key_gen()?;` — this **persists the ETH sk to disk** (`eth_key_gen` → `save_eth_key` → `write_eth_key`, `src/crypto/eth_keys.rs:17-20,87-95`).
  - `:56` `AttestationEvidence::new(&payload).await?` — the CVM-agent round-trip happens **after** the sk is on disk.
  - If the CVM call errors / hangs / the process is killed between `:31` and the `Ok` returned by the handler (`attest_fresh_eth_key_with_blockhash.rs:14-16`), the caller (reef) gets a 500 / connection reset / nothing, **but the ETH sk file is already on disk**. reef never learns this pubkey exists.
- **validate-custody orphan window:** `src/enclave/guardian/mod.rs`
  - `:82-85` `write_bls_key(...)` persists the decrypted BLS share.
  - `:88-95` `approve_custody(...).await` produces the signature returned to the caller.
  - If `approve_custody` errors (`bail!("Failed to sign correctly")` at `:346`, or `guardian_module_address.parse()?` at `:323`) or the process is killed between `:85` and the handler's `Ok` (`validate_custody.rs:10`), the **BLS share is persisted but no approval is returned**. reef may skip-provision.
- No rollback, no reconciliation, no GC found anywhere (`delete_eth_key`/`delete_bls_key` exist in `key_management.rs:84-95` but are **never called from any guardian route** — confirmed sut-analysis §4).
- Consumer behavior: reef's `validate_custody` / `attest_fresh_eth_key` (`src/client/guardian.rs:26-92`) treat any non-2xx (or transport error) as `Err` and bubble it up; the orchestrator (reef-guardian `new_registration.rs`) then skips/aborts — so an orphaned share on disk is a secret that exists but is **detached from any provisioning record**.

## What breaks

- **Accumulation:** every failed/killed keygen leaves a dead ETH key file; every failed/killed validate-custody (after the write) leaves a dead BLS share. Across retries these accumulate unboundedly in `./data/keys/{eth,bls}_keys/` (ties to Resource Boundaries: unbounded-keydir-growth.md).
- **Security/liveness coupling (sut-analysis W10):** an orphaned BLS share is still a **live exit-signing capability** — `sign-exit` (`mod.rs:352-370`) will happily sign for any stored share with zero authorization, even though the custody approval that should have gated it was never returned. So a write-before-success orphan converts a *failed* custody into a *usable* exit-signer that the protocol doesn't know about.
- **Reconciliation gap:** there is no endpoint or process to answer "for every stored share, is there a corresponding returned approval?" — so a partial failure is permanent and invisible.

## Antithesis angle

- Workload: issue validate-custody requests, some of which are made to fail *after* the write (inject CVM/RPC fault for the verifying path, or kill the node between `:85` and the response). Then enumerate `GET /eth/v1/keygen` (lists eth keys) and the bls keys dir, and the set of approvals reef actually received.
- **Sometimes / liveness property:** `Sometimes(a returned approval exists for a stored share)` is the healthy state; the interesting failure is the persistent orphan. Best expressed as an SUT-instrumented invariant counting (shares written) vs (approvals returned), checked after a `ANTITHESIS_STOP_FAULTS` quiet window — `eventually_` every persisted share is either matched by a returned approval or removed.
- Concrete reachability assertion: `Reachable("validate-custody persisted share then failed to return approval")` — proves the orphan window is real under fault.

## Fault dependency

- The *crash-between-write-and-response* variant **REQUIRES node-termination (kill/restart) — DISABLED by default.**
- The *error-after-write* variant does **not** require termination: it can be driven by making `approve_custody` fail (e.g. malformed `guardian_module_address` so `parse()?` at `:323` errors) — that error path runs after the BLS write at `:85`. So this property has a fault-free reachability proof AND a termination-dependent crash variant. Flag both.

## Timing / config deps

- For keygen orphans, easiest with CVM unavailable/erroring (no `CVM_AGENT_STUB`, no socket) so `AttestationEvidence::new` at `:56` fails after the `:31` write.
- For validate-custody orphans via error: a request that passes verify_custody (real share) but has a bad `guardian_module_address` reaches the write then fails `approve_custody`. Requires a workload that can produce a valid keyshare payload — non-trivial; the kill-based variant is easier to trigger generically.

## Instrumentation status: MISSING (SUT-side required)

- Need counters/events emitted at `mod.rs:85` (share-persisted, with the share pk) and at the handler success return (approval-returned, same pk), so the invariant "persisted ⊆ approved (eventually)" can be asserted. `assert_always` would be wrong (the window legitimately exists transiently) — use `assert_reachable` for the orphan window + an `eventually_`/`finally_` liveness assertion for reconciliation after faults stop.
- antithesis-sdk not yet a dependency.

## Open questions (why they matter)

- Is there any out-of-band reconciliation/GC for `./data/keys` (cron, startup sweep)? None found in this repo. **Matters: if none, orphans are permanent and the resource + security exposure is real.** (sut-analysis Open Q6.)
- Does reef ever retry the *same* keygen/custody such that a new file is written and the orphan is left behind, or does it reuse? Cross-repo. **Matters for accumulation rate and for whether an orphaned share could later be selected by sign-exit.**
- Is an orphaned BLS share reachable by sign-exit in the deployed topology (is validate-custody/sign-exit exposed to anyone but reef)? sut-analysis Open Q4. **Matters: turns a liveness/resource issue into a security one.**

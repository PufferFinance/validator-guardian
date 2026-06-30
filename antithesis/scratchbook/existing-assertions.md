---
sut_path: /home/fawad/puffer/projects/validator-guardian
commit: f7dbd88d99de21a6d8caba5f7de8216624ab1098
updated: 2026-06-30
external_references:
  - path: /home/fawad/puffer/projects/reef
    why: Primary consumer of the guardian (reef-guardian) via GuardianClientTrait — reveals real request patterns (notably verify_session=false today).
  - path: /home/fawad/puffer/projects/coral
    why: Drives the validator enclave / BLS keygen + custody flow that feeds guardian's validate-custody endpoint.
---

# Existing Antithesis SDK Assertions

## Summary

**No Antithesis SDK assertions exist in this codebase.** This is a greenfield
instrumentation target. The workload built by `antithesis-workload` will be the
first source of Antithesis assertions, and SUT-side instrumentation (where a
property calls for it) will need to be added from scratch.

## Scan Methodology

Searched the entire repository (excluding `target/`) for:

- Antithesis SDK imports / crate references: `antithesis` in any `.rs` or `.toml`
  file, and in `Cargo.lock`.
- Assertion macros / functions: `assert_always`, `assert_sometimes`,
  `assert_reachable`, `assert_unreachable` (and case-insensitive variants).

```
grep -rniE "assert_always|assert_sometimes|assert_reachable|assert_unreachable|antithesis" \
  --include=*.rs --include=*.toml .            # → 0 matches outside target/
grep -i "antithesis" Cargo.lock                # → 0 matches
```

## Findings

| Category | Result |
|---|---|
| Antithesis Rust SDK (`antithesis-sdk`) in `Cargo.toml` / `Cargo.lock` | **Absent** |
| `assert_always!` / `assert_always_or_unreachable!` | **None** |
| `assert_sometimes!` | **None** |
| `assert_reachable!` / `assert_unreachable!` | **None** |
| `lifecycle::*` (setup_complete, send_event) | **None** |
| `random::*` (Antithesis-guided randomness) | **None** |

## Implications for downstream skills

- The Antithesis Rust SDK (`antithesis-sdk` crate) must be added as a dependency
  before any SUT-side instrumentation can compile. It is designed to be a no-op
  / low-overhead outside the Antithesis environment, so it is safe to leave in
  production builds of the guardian binary.
- Standard Rust `assert!`/`debug_assert!`, `anyhow::bail!`, and the `bail!`-based
  validation throughout `verify_session_evidence`, `verify_custody`,
  `verify_deposit_message`, and `approve_custody` are **not** Antithesis
  assertions — they return errors (HTTP 500) rather than reporting property
  outcomes. Several of these error paths are strong candidates for being mirrored
  as Antithesis `Always`/`Unreachable`/`Sometimes` assertions; the property
  catalog notes these per-property.
- Because nothing is instrumented yet, every property in the catalog should be
  treated as "instrumentation missing" unless the evidence file says otherwise.

## Existing non-Antithesis test instrumentation (for context)

These are ordinary Rust tests, not Antithesis assertions, but they document the
invariants the developers already care about and are useful seeds for properties:

- `src/enclave/guardian/mod.rs` `#[cfg(test)]`: `test_verify_custody_with_success`,
  `test_verify_custody_with_fail`, `test_verify_deposit_message`,
  `test_approve_custody`, `test_sign_vem`, `test_setup_valid`.
- `src/io/key_management.rs` `#[cfg(test)]`: write/read/delete/list round-trips
  for ETH and BLS keys.
- `src/client/tests/`, `tests/signing_tests/`: client/integration round-trips
  (note: these run with `verify_session = false` / `do_remote_attestation =
  false`, so the on-chain attestation path is never exercised by existing tests).

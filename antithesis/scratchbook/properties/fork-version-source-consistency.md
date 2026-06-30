# Property: deposit domain uses a fork_version that is consistent with the trusted genesis

slug: fork-version-source-consistency
focus: (10) Version Compatibility (W13) + (4) Protocol Contracts (deposit domain)
priority: MEDIUM-HIGH
confidence: HIGH (the inconsistency is confirmed in code + reef)

## Origin / the inconsistency
The guardian computes the BLS **deposit domain** from a fork_version, and there
are TWO sources of fork_version in the binary:

1. **Custody / deposit path uses the REQUEST-supplied fork_version**
   (untrusted, caller-controlled):
   - `src/enclave/types.rs` `BlsKeygenPayload.fork_version` (field, line 146).
   - Used in `deposit_message_root()` (types.rs:175-192, line 186:
     `compute_domain(DOMAIN_DEPOSIT, Some(self.fork_version.clone()), None)`) and
     transitively in `deposit_data_root()` (types.rs:194-203).
   - These feed `verify_deposit_message` (mod.rs:260-279, S1/S2) and the
     `approve_custody` preimage (the BLS signature & dd_root that get co-signed).

2. **Voluntary-exit / BLS signing path uses the TRUSTED AppState genesis**
   (set once at startup from `GENESIS_FORK_VERSION` env):
   - `src/enclave/shared/mod.rs::sign_validator_message` line 69:
     `req.to_signing_root(Some(state.genesis_fork_version))`.
   - (This route is the validator/secure-signer path, but it documents the
     trusted-source pattern the custody path diverges from.)
   - Note `sign_voluntary_exit_message` (mod.rs:352-370) uses
     `req.fork_info` from the request too — also caller-supplied.

So: custody/deposit verification trusts `fork_version` from the payload; the
signing path uses the genesis from trusted config. W13 = inconsistent trust.

## What reef sends
reef populates `fork_version: fork_info.genesis_fork_version` from its OWN
trusted network config (new_registration.rs:262). So in the honest flow the
request fork_version EQUALS reef's genesis — they happen to agree. But the
guardian does NOT cross-check it against its own `AppState.genesis_fork_version`;
it blindly trusts the request value.

## What breaks on drift / mismatch
- If the request `fork_version` differs from the fork version actually used by the
  validator enclave at BLS-signing time (e.g. reef config drift across a
  fork/upgrade, a stale BFF, or a malicious caller), `verify_deposit_message`
  recomputes the deposit domain with the WRONG fork_version. Outcomes:
  - Mismatched vs the validator's signature → `bail!("DepositMessage signature
    invalid")` (S1) → custody refused (liveness; fail-closed). Most likely.
  - In a crafted scenario where the attacker controls both the signature and the
    fork_version, they could produce a *self-consistent but wrong-domain* deposit
    that the guardian co-signs — a "valid-looking but wrong deposit domain"
    approval (the user's stated W13 hazard). The on-chain BeaconDepositContract
    uses the real domain, so a wrong-domain deposit ultimately fails at deposit
    time, but the guardian has already co-signed and persisted the share (F9
    orphan).
- Version-compat dimension: across a hard fork the genesis/fork version changes;
  if the guardian binary, reef config, and validator enclave don't all update in
  lockstep, deposits silently fail to verify.

## Property statement
- `Always`: the fork_version used to compute the deposit domain equals the
  guardian's trusted `AppState.genesis_fork_version` (i.e. the guardian SHOULD
  reject or at least the property should assert the request value matches the
  trusted value). Today there is NO such check — so the property surfaces a real
  gap.
- Weaker / behavioral: `Always(if request.fork_version != genesis_fork_version
  then custody is refused)` — assert the binary never co-signs a deposit built
  with a fork_version other than the trusted one.

## Antithesis angle
- Input-space coverage: generate requests with `fork_version` equal to and
  different from the configured genesis; assert no approval is produced for a
  mismatched fork_version (or that mismatches always fail-closed at S1).
- `Sometimes(request.fork_version != genesis)` to prove the divergent-input branch
  is reachable, then `Unreachable(co-signed approval with mismatched fork_version)`.
- This is partly fault-driven (config drift can be modeled as a clock/version
  fault around a fork boundary) and partly deterministic input coverage.

## Instrumentation status: MISSING
No assertion ties request fork_version to AppState genesis. The guardian's
`verify_and_sign_custody_received` does not even have access to AppState today
(it's a free fn taking only the request) — instrumentation would require threading
`genesis_fork_version` in, OR asserting at the deposit-domain computation site.

## Open questions
- Is the custody path *intended* to trust the request fork_version (to support
  multiple networks per guardian), or is that an oversight vs the signing path's
  trusted-genesis pattern? **Needs product/design input** — determines whether
  this is "assert equality" (bug) or "assert membership in an allowed set"
  (feature).
- Across a fork, who is the source of truth for fork_version — reef config, the
  validator enclave at keygen, or the guardian's GENESIS_FORK_VERSION env? They
  are three independently-deployed components (version-compat blast radius).

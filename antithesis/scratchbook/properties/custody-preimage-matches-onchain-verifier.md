# Property: Rust custody-approval preimage byte-matches the on-chain GuardianModule verifier

slug: custody-preimage-matches-onchain
focus: (4) Protocol Contracts — signature-correctness core (S6 / W0)
priority: CRITICAL
confidence: HIGH (cross-repo confirmed; this is a *live, currently-broken* drift)

## Origin / where the bytes are built
- Rust producer: `src/enclave/guardian/mod.rs`
  - `approve_custody()` lines **312-350**. ABI preimage at **322-336**:
    ```
    ethers::abi::encode(&[
      Token::Address(guardian_module_address),   // field 1
      Token::Uint(U256::from(chain_id)),          // field 2
      Token::Uint(U256::from(validator_index)),   // field 3
      Token::Bytes(pk_set.public_key().to_bytes()),          // field 4  blsPubKey (48B)
      Token::Bytes(withdrawal_credentials),       // field 5  (32B)
      Token::Bytes(signature.to_bytes()),         // field 6  BLS sig (96B)
      Token::FixedBytes(deposit_data_root),       // field 7  (bytes32)
    ])
    ```
    then `keccak256(...)` (sha3::Keccak256, line 319/338) and EIP-191
    `wallet.sign_message(&msg_to_be_signed)` (line 342), then self-recover check
    (S5, lines 345-347).
- Field source: `verify_and_sign_custody_received()` (mod.rs:88-95) passes
  `validator_index`, `guardian_module_address`, `chain_id` straight from the
  request (`ValidateCustodyRequest`, types.rs:85-93). `chain_id` and
  `guardian_module_address` are **caller-controlled**.

## External verifier it must byte-match
On-chain `GuardianModule.validateProvisionNode(...)` →
`LibGuardianMessages._getBeaconDepositMessageToBeSigned(...)` →
`keccak256(abi.encode(...)).toEthSignedMessageHash()`, recovered against each
guardian's stored `enclaveAddress` in `validateGuardiansEnclaveSignatures` →
`_validateSignatures` (uses ECDSA tryRecover, GuardianModule.sol:234-240, 381-385).

The EIP-191 side is consistent: Rust `sign_message` == Solidity
`.toEthSignedMessageHash()` + raw ECDSA recover. **The field set is NOT.**

## W0 — THE DRIFT (confirmed against the actual checkout)
`puffer-contracts` checkout is on branch **`feat/tdx`** (same TDX line as the
guardian). Two relevant branches:

| Branch | `_getBeaconDepositMessageToBeSigned` preimage | matches Rust 7-field? |
|---|---|---|
| **`feat/tdx`** (current HEAD `e897966`) | `abi.encode(pufferModuleIndex, pubKey, withdrawalCredentials, signature, depositDataRoot)` — **5 fields**, `pure` | **NO** |
| `origin/fix/increase-guardian-signatures-security` | `abi.encode(verifyingContract, block.chainid, pufferModuleIndex, pubKey, withdrawalCredentials, signature, depositDataRoot)` — **7 fields**, `view` | **YES (exact order match)** |

`src/LibGuardianMessages.sol:25-34` (feat/tdx) = 5-field; the diff to
`fix/increase-guardian-signatures-security` adds `verifyingContract` +
`block.chainid` as the first two fields (verified via `git diff`).

Rust field order vs `fix/...` field order:
| # | Rust (ethers Token) | fix-branch (abi.encode) | match |
|---|---|---|---|
| 1 | Address(guardian_module_address) | verifyingContract (address) | ✓ (caller must pass `address(this)`) |
| 2 | Uint(chain_id) | block.chainid | ✓ (caller must pass true chainid) |
| 3 | Uint(validator_index) | pufferModuleIndex | ✓ |
| 4 | Bytes(blsPubKey) | pubKey | ✓ |
| 5 | Bytes(withdrawal_credentials) | withdrawalCredentials | ✓ |
| 6 | Bytes(BLS signature) | signature | ✓ |
| 7 | FixedBytes(deposit_data_root) | depositDataRoot | ✓ |

NOTE field-3 semantic: contract field is `pufferModuleIndex` (the *module*
index); reef sends `validator_index: payload.puffer_module_index`
(reef new_registration.rs:267) so the name "validator_index" in the guardian is
actually the puffer module index — consistent, just mis-named.

## What breaks on drift
- **Against `feat/tdx` today**: the guardian's 7-field signature recovers to a
  DIFFERENT address than its enclaveAddress → `validateProvisionNode` reverts
  `Unauthorized()` → **provisioning of every validator stalls** (liveness break,
  protocol-wide because all guardians run the same binary). This is the
  end-to-end break the user flagged.
- Commit `61bf1d2` ("use correct payload") + `d119699` (added module addr +
  chainId) prove this preimage already drifted at least once.

## Property statement (deterministic correctness, NOT fault-driven)
For any valid custody request, the digest the guardian signs MUST equal
`keccak256(abi.encode(...))` computed with the deployed contract's field set and
order, and the recovered signer MUST equal the guardian's enclaveAddress.

Best expressed as an **oracle/differential test**: re-implement the on-chain
preimage with `ethers::abi::encode` (or `alloy::sol!`/foundry) and assert
`Always(rust_digest == contract_digest)`. Antithesis adds value via **input-space
coverage** (diverse chainId, validatorIndex, address, pubkey/sig/wc lengths,
0x-prefixed vs not), and by catching a *recurrence* of the drift on future
commits — it is mostly a deterministic-correctness property, not a fault-injection
one. Call this out honestly.

## Antithesis angle
- Generate varied `(chainId, validatorIndex, guardian_module_address, blsPubKey,
  wc, depositSig, ddRoot)`; assert the produced signature verifies under a
  reference Solidity-equivalent encoder. `Always` (must hold for every input).
- A second `Sometimes`/`Reachable` flavor: assert at least one full
  produce→verify round-trip *succeeds* against the pinned contract preimage — if
  this is never Reachable, the binary and the deployed contract are misaligned
  (exactly W0).

## Instrumentation status: MISSING
No Antithesis SDK in repo. SUT-side: need (a) a reference encoder of the
authoritative on-chain preimage, (b) an `assert_always` that the recovered signer
== expected enclave address for generated inputs. Existing `test_approve_custody`
(mod.rs:570-581) only asserts `.is_ok()` — it never checks the bytes, so it would
NOT catch the drift. This is the single highest-value, lowest-coverage property.

## Open questions (why they matter)
- **(needs human input — deployed bytecode)** Which contract branch/commit is the
  deployment actually on? **Code-level analysis is now fully resolved** (see
  Investigation Log Q3): only `origin/fix/increase-guardian-signatures-security`
  carries the 7-field verifier that byte-matches `approve_custody`; the
  checked-out `feat/tdx` (HEAD `e897966`) and `master` carry the legacy 5-field
  preimage. So IF the deployment tracks `feat/tdx`, custody is broken now; IF it
  tracks `fix/...`, custody matches but rotate-key has no on-chain verifier. **No
  single branch satisfies both.** Which bytecode is on the target chain remains
  human-input.
- Is `guardian_module_address`/`chain_id` validated anywhere before signing? No —
  fully caller-trusted (W2). A reference encoder must use the *true* values the
  contract uses (`address(this)`, `block.chainid`), so a caller passing a wrong
  address yields a valid-looking but unverifiable signature.

### Investigation Log

#### Q3: 5-field vs 7-field custody preimage — confirm the drift against the contract
- **Examined:** `git -C ~/puffer/projects/puffer-contracts branch --show-current`
  (= `feat/tdx`, HEAD `e897966`), `branch -a`, `log --all -- '**/LibGuardianMessages.sol'`,
  `git show <branch>:...`. Files: `mainnet-contracts/src/LibGuardianMessages.sol`
  `_getBeaconDepositMessageToBeSigned`; `mainnet-contracts/src/GuardianModule.sol`
  `validateProvisionNode` / `validateGuardiansEnclaveSignatures` / `_validateSignatures`.
  Rust side: `src/enclave/guardian/mod.rs::approve_custody` (:322-336).
- **Found:**
  - `feat/tdx` (current HEAD) `LibGuardianMessages.sol:25-34`: **5 fields**,
    `keccak256(abi.encode(pufferModuleIndex, pubKey, withdrawalCredentials,
    signature, depositDataRoot)).toEthSignedMessageHash()`, `internal pure`. NO
    `verifyingContract`, NO `chainid`. → does NOT match Rust 7-field.
  - `origin/fix/increase-guardian-signatures-security`: **7 fields**, in-hash order
    `verifyingContract, block.chainid, pufferModuleIndex, pubKey,
    withdrawalCredentials, signature, depositDataRoot`, `internal view`,
    `.toEthSignedMessageHash()`. → EXACT field-set/order match to the Rust tuple
    (Address, Uint chainId, Uint validatorIndex/pufferModuleIndex, Bytes blsPubKey,
    Bytes withdrawalCredentials, Bytes blsSignature, FixedBytes depositDataRoot).
  - Consumer: `GuardianModule.validateProvisionNode` → `validateGuardiansEnclaveSignatures`
    → `_validateSignatures` uses `ECDSA.tryRecover(signedMessageHash, sig)` and
    compares `currentSigner == signers[i]` (the guardian `enclaveAddress`), with
    empty enclave addresses mapped to `0x..dEaD` to defeat the recover-to-zero
    attack. EIP-191 side consistent (Rust `wallet.sign_message` ==
    `.toEthSignedMessageHash()` + raw ECDSA recover).
- **Not found:** Any branch that has BOTH the 7-field custody preimage AND the
  `ROTATE_GUARDIAN_KEY` rotate path. `ROTATE_GUARDIAN_KEY` exists only on `feat/tdx`.
- **Conclusion:** RESOLVED (code). The W0 drift is real and exactly as the file
  states. The 7-field guardian preimage byte-matches ONLY
  `origin/fix/increase-guardian-signatures-security`; the checked-out `feat/tdx`
  and `master` are 5-field and will reject the guardian's signature
  (`Unauthorized()` / provisioning stall). Which is deployed = human-input.
  Property UNCHANGED (still CRITICAL, still a live drift at the code level).


---

> **[merged]** consolidated from discovery focus file `prop-focus-6/coordination-approval-preimage-matches-onchain-verifier.md`

# Property: a honest guardian's custody-approval signature is accepted by the on-chain GuardianModule verifier (agreement)

Slug: `coordination-approval-preimage-matches-onchain-verifier`
Focus: (7) Distributed Coordination — threshold agreement (on-chain)
Instrumentation: **MISSING** (and partly OUT-OF-PROCESS — see fault/OQ)

## Origin / the coordination assumption
Architecture (confirmed cross-repo, corrects the "aggregated off-chain by reef"
framing in sut-analysis §1/§13):
- The guardian set is **edge-distributed**. Each guardian runs its own
  reef-guardian + enclave and submits its OWN approval **directly on-chain**.
  `reef-guardian/.../provision_validator.rs:180-215`:
  `enclave_signatures = vec![enclave_signature]` (a SINGLE sig) →
  `puffer_protocol_contract.provision_node(enclave_signatures, keydata_signature, deposit_root)`.
- There is **no off-chain aggregation** and **no off-chain quorum check on the
  provisioning path**. The on-chain `PufferProtocol`/`GuardianModule` contract is
  the aggregator and threshold enforcer. (reef's `verify_signatures_number`
  threshold check, `reef-guardian/src/utils/contract.rs:92-118`, exists but is
  used for batch **withdrawals**, not provisioning.)
- Threshold itself is read from `GuardianModule.get_threshold()`
  (`reef-guardian/src/utils/contract.rs:74-75`).

The agreement property: **each honest guardian, given the same valid keygen
payload, must produce a co-signature that the on-chain verifier accepts** — so
that any M honest guardians clear the threshold. Because all guardians run the
**same binary**, agreement reduces to a single question: does THIS binary's
signed preimage byte-match the contract's verifier preimage?

## Files / functions / lines (the guardian's signed preimage)
- `src/enclave/guardian/mod.rs:312-350` `approve_custody`. The signed message is
  `keccak256(abi.encode(...))` over a **7-field** ABI tuple in fixed order
  (`:322-336`):
  1. `guardian_module_address` (Address)
  2. `chain_id` (Uint)
  3. `validator_index` (Uint)
  4. `bls_pub_key` (Bytes, = `public_key_set().public_key()`)
  5. `withdrawal_credentials` (Bytes)
  6. `deposit_signature` (Bytes, = BLS `signature()`)
  7. `deposit_data_root` (FixedBytes)
- Self-recovery check `:345-347` `bail!("Failed to sign correctly")` — guarantees
  the sig recovers to the enclave's own address (S5), i.e. internally consistent.
- keygen rotate-key preimage (separate verifier path):
  `src/enclave/guardian/mod.rs:38-56` —
  `keccak256(abi.encode("ROTATE_GUARDIAN_KEY", addr, chainId, blockNumber, pubKey))`.

## What breaks / the risk (W0)
sut-analysis §9 W0 (flagged, cross-repo, **needs human confirmation**): on the
deployed `puffer-contracts` branches, `LibGuardianMessages.sol` reportedly still
verifies the **old 5-field** approve preimage, while this guardian signs the new
**7-field** preimage (commit `d119699` added `guardian_module_address`+`chain_id`;
`61bf1d2` shows the preimage was already wrong once). If true:
- **A correct quorum of honest approvals is REJECTED on-chain** → provisioning
  stalls forever (liveness/agreement break) regardless of how many guardians
  agree. This is a *correlated systematic* failure: all guardians use the same
  (wrong) preimage, so it is not tolerated below threshold — it fails the WHOLE
  set.
- Worse: the comment at `mod.rs:321` says
  `// validatorIndex, pubKey, withdrawalCredentials, signature, depositDataRoot`
  (5 fields) but the code encodes 7 — the comment matches the OLD format,
  evidence the drift is real and unreconciled.

The matching also extends to `chain_id` and `validator_index` being
**caller-supplied** (sut-analysis W2/W3): the guardian commits to whatever
`chain_id`/`validator_index` the request carries, so agreement also requires every
guardian to be handed the SAME chain_id/validator_index by their orchestrator.

## Antithesis angle
This property's truth lives in the **contract**, which is NOT in this SUT, so
it is only partially testable in-process:
- IN-PROCESS (testable now): `Always("approve_custody output is a 65-byte ECDSA
  sig that recovers to the enclave wallet address")` — mirrors the `:345` check as
  an `assert_always`. Plus `Always("approve_custody preimage encodes exactly 7
  ABI fields in the fixed order")` — a structural assertion guarding against
  silent reordering/field-count regressions (the exact thing that churned in
  `d119699`/`61bf1d2`).
- DETERMINISM/AGREEMENT (testable now): `Always("two guardians given byte-identical
  keygen payloads + chain_id + validator_index + module address produce the
  byte-identical signed preimage")` — the signed message must be a pure function
  of those inputs (it is: no nonce, no randomness, no per-guardian binding — see
  sut-analysis W1). This is the in-process proxy for "the set agrees."
- CROSS-PROCESS (the real W0): needs the contract in the harness — assert that a
  threshold set of these signatures is ACCEPTED by a deployed `GuardianModule`.
  **Requires the contract branch answer + a contract instance in the workload.**

## Fault dependency
- No network/clock/restart fault needed for the in-process determinism/structural
  assertions.
- The full on-chain agreement check needs an EVM (anvil fork) of the **correct
  contract branch** in the test environment — this is an environment/topology
  dependency, not an Antithesis fault.

## Open questions (all gate the cross-process half of this property)
- **W0 / OQ1:** which `puffer-contracts` branch/commit verifies the guardian
  approval in the target deployment, and does it expect 5 or 7 fields? **Code-level
  RESOLVED (see Investigation Log Q3 in the section above):** 7-field verifier =
  `origin/fix/increase-guardian-signatures-security` only; `feat/tdx`/`master` =
  legacy 5-field. The remaining unknown — which bytecode is *deployed* on the
  target chain — **needs human input.** If 5-field is live, custody approval is
  broken end-to-end today.
- **OQ2:** production M and N. Determines how many correlated-correct sigs are
  needed and the blast radius of a systematic preimage bug. **Needs human input.**
- Does `provision_node` verify each guardian sig against a registered guardian
  address set (so a non-enclave signer is rejected)? Confirms the on-chain side of
  W4 (guardian identity never pinned in-enclave).

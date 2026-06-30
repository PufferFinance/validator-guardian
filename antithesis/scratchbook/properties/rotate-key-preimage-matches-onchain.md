# Property: ROTATE_GUARDIAN_KEY preimage byte-matches on-chain GuardianModule.rotateGuardianKey

slug: rotate-key-preimage-matches-onchain
focus: (4) Protocol Contracts (S7) + (10) Version Compatibility (CVM agent dev-branch dep)
priority: HIGH
confidence: HIGH (byte-aligned on feat/tdx; verified Rust + Solidity + atakit signer)

## Origin / where the bytes are built
- Rust producer: `src/enclave/guardian/mod.rs::attest_new_eth_key_with_blockhash`
  lines **21-58**. Preimage at **45-51**:
  ```
  ethers::abi::encode(&[
    Token::String("ROTATE_GUARDIAN_KEY"),       // field 1
    Token::Address(address),                     // field 2  guardian module addr
    Token::Uint(U256::from(chain_id)),           // field 3
    Token::Uint(U256::from(block_number)),       // field 4
    Token::Bytes(pk.serialize().to_vec()),       // field 5  uncompressed 65B secp256k1 pubkey
  ])
  ```
  Then `AttestationEvidence::new(&payload)` (line 56) →
  `io/remote_attestation.rs:30-43` → `io/cvm_agent.rs::sign_with_session` →
  `CvmAgent::sign_message(payload)`. The CVM agent computes `keccak256(payload)`
  and signs the hash with the session key (raw ECDSA, **no EIP-191 prefix**).
  Confirmed in atakit `crates/automata-cvm-agent/src/sim/state.rs::sign()`:
  `let hash = keccak256(message); session.session_signing_key.sign_hash(&hash)`.

## External verifier it must byte-match
On-chain `GuardianModule.rotateGuardianKey(...)`,
`mainnet-contracts/src/GuardianModule.sol:334-338` (branch `feat/tdx`):
```
bytes32 signedMessageHash =
    keccak256(abi.encode("ROTATE_GUARDIAN_KEY", address(this), block.chainid, blockNumber, pubKey));
bool isValid = SESSION_REGISTRY.verifySessionSignature(
    proof.sessionId, proof.sessionKey, signedMessageHash, proof.signature);
```
`verifySessionSignature` → `signatureVerifier.verify(sessionKey, message, signature)`
over the **raw** `signedMessageHash` (atakit SessionRegistry.sol:411-426, 703-705 —
no `toEthSignedMessageHash`). So the verified message is exactly
`keccak256(abi.encode(...))`, matching what the CVM agent signed. ✓ byte-aligned.

Field-by-field (Rust vs feat/tdx contract):
| # | Rust | contract | match |
|---|---|---|---|
| 1 | String("ROTATE_GUARDIAN_KEY") | "ROTATE_GUARDIAN_KEY" | ✓ |
| 2 | Address(guardian_module_address) | address(this) | ✓ (caller must pass module addr) |
| 3 | Uint(chain_id) | block.chainid | ✓ |
| 4 | Uint(block_number) | blockNumber | ✓ |
| 5 | Bytes(pk.serialize() = uncompressed 65B) | pubKey (require length == _ECDSA_KEY_LENGTH) | ✓ if _ECDSA_KEY_LENGTH == 65 |

## W0 dual-bind problem (the open contract question)
`feat/tdx` HAS `rotateGuardianKey` with SessionRegistry verification (MATCH).
`origin/fix/increase-guardian-signatures-security` has **no `ROTATE_GUARDIAN_KEY`
at all** (grep returns nothing) — it still uses the older attestation format.
Therefore:
- `feat/tdx`: rotate-key MATCHES, custody DRIFTS (5 vs 7 fields).
- `fix/...`: custody MATCHES, rotate-key is the old format (would NOT match this
  guardian's SessionRegistry-based rotate).
**No single deployed contract branch byte-matches BOTH guardian preimages
simultaneously.** This is the core unresolved W0 risk.

## What breaks on drift
- Field/order/encoding mismatch (e.g. compressed vs uncompressed pubkey, missing
  chainId, wrong addr) → `verifySessionSignature` returns false →
  `rotateGuardianKey` reverts `InvalidSignature()` → guardian can never register
  its enclave key on-chain → that guardian is **excluded from quorum** (liveness;
  if it hits ≥ N-M+1 guardians, provisioning halts permanently).

## Version-compatibility hazard (focus 10)
`automata-cvm-agent = { git=...atakit, branch = "dev" }` is **unpinned to a
branch**, Cargo.lock currently pins commit `9f9bfea`. If the agent's signing
semantics change (e.g. starts EIP-191-wrapping, or hashes with sha256 instead of
keccak256 — note SessionRegistry's *registration* messages use `sha256`,
SessionRegistry.sol:269), the rotate-key signature silently stops verifying. The
property protects against atakit drift, not just our own code.

## Property statement
For generated `(chain_id, block_number, guardian_module_address, fresh pubkey)`,
the digest the CVM agent signs (`keccak256(abi.encode(...))`) MUST equal the
contract's `signedMessageHash`, AND the resulting session signature MUST verify
under SessionRegistry.verifySessionSignature for the same digest.

## Antithesis angle
- Mostly deterministic-correctness / input-coverage: vary chain_id, block_number,
  module address; assert `Always(rust_keccak == contract_keccak)` against a
  reference encoder. Antithesis's value is diverse-input generation + regression
  catching on atakit/contract bumps, NOT fault injection.
- With `CVM_AGENT_STUB` unset and a sim agent: `Reachable`/`Sometimes(verify ok)`
  — at least one keygen→on-chain-verify round-trip succeeds. If never reachable,
  the agent/contract are misaligned.
- Fault angle that IS real: CVM agent socket hang/no-timeout (F3) — keygen blocks
  forever; covered better in a liveness property, noted here as adjacent.

## Instrumentation status: MISSING
No SDK. Needs a reference encoder + assertion. The existing path has zero tests
that touch the real preimage bytes against the contract.

## Open questions
- ~~**Is `_ECDSA_KEY_LENGTH == 65`?**~~ **RESOLVED (code).** `feat/tdx`
  GuardianModule.sol:37 `uint256 internal constant _ECDSA_KEY_LENGTH = 65;`.
  rotateGuardianKey additionally requires `ownerKey.typeId == ALGO_ID_ES256K`
  (:316) and `ownerKey.key.length == 65` (:317) and `pubKey.length == 65`
  (:325-327). Field-5 65-byte uncompressed (`pk.serialize()`) matches.
- ~~Does the production CVM agent sign `keccak256` (like the sim)?~~ **RESOLVED
  for the sim/code path; (partial: real DCAP binary not in-repo).** Every
  `/sign-message` implementation present in atakit (both pinned commit `9f9bfea`
  and checkout `797090c`) hashes raw `keccak256(message)` then `sign_hash` with
  NO EIP-191 prefix (`automata-cvm-agent/src/sim/state.rs::sign()`). Grep for
  `eip191|personal_sign|Ethereum Signed Message|\x19` across the cvm-agent crate
  and `automata-tee-workload-measurement-0.1.4` returns NOTHING. The only
  server-side `/sign-message` handler in atakit is the sim; the production DCAP
  agent that runs inside the TDX VM is an external Automata binary NOT in any of
  the three available repos — its byte behavior cannot be code-verified here.
  NB: the workload-measurement `stubs::sign_message` free fn uses **SHA-256**
  (`stubs.rs:159-189`), but that is the *session-registration* signer, NOT the
  per-message `/sign-message` path the guardian uses — do not conflate them.
- **(needs human input)** Which contract branch is actually DEPLOYED? Code-level
  answer (see W0 below) is now fully pinned: only `feat/tdx` has rotateGuardianKey
  at all, and its preimage byte-matches. But which bytecode is on the target chain
  is human-input.

### Investigation Log

#### Q1: Does the Automata CVM agent sign `keccak256(payload)` raw (no EIP-191 prefix)?
- **Examined:** atakit `crates/automata-cvm-agent/src/sim/state.rs::sign()` (both
  local HEAD `9f9bfea` = the Cargo.lock-pinned commit, and cargo checkout
  `797090c`); `client/cvm_agent.rs::sign_message` (the client the guardian calls);
  `sim/server.rs` (`/sign-message` route → `handle_sign` → `state.sign`);
  `automata-tee-workload-measurement-0.1.4/src/stubs.rs::sign_message`; full grep
  for EIP-191 markers across cvm-agent + workload-measurement. SUT call chain:
  `mod.rs:56 AttestationEvidence::new` → `remote_attestation.rs::new` →
  `cvm_agent.rs::sign_with_session` → `CvmAgent::sign_message`.
- **Found:** `sim/state.rs::sign()` = `let hash = keccak256(message);
  session.session_signing_key.sign_hash(&hash)` — raw keccak256, returns 65-byte
  r‖s‖v. Identical bytes in `9f9bfea` and `797090c`. Client doc comment
  ("hashed with keccak256 before signing") agrees. No EIP-191 anywhere.
- **Not found:** Any non-sim/"real" server implementation of `/sign-message`
  inside atakit (only the sim). Any EIP-191/personal_sign/`\x19` prefix.
- **Conclusion:** RESOLVED at code level — the guardian's assumption (raw
  keccak256, no prefix) is correct for every signer present in the available
  source. The contract side agrees: `SessionRegistry.verifySessionSignature`
  verifies over the raw `bytes32 message` with NO re-hash
  (SessionRegistry.sol:411-427), and rotateGuardianKey passes a raw keccak256
  digest (NO `toEthSignedMessageHash`). The single residual is the closed-source
  production DCAP agent, which is human-input. Property UNCHANGED (still
  byte-aligned); confidence strengthened.

#### Q2: `_ECDSA_KEY_LENGTH` / session key encoding in the contract
- **Examined:** puffer-contracts `feat/tdx` GuardianModule.sol (constants +
  rotateGuardianKey), `IGuardianModule.sol` (GuardianSessionProof struct),
  workload-measurement submodule `types/Common.sol` (PublicIdentity),
  `types/Constants.sol` (ALGO_ID_*), `SessionRegistry.sol::verifySessionSignature`,
  `SignatureVerifier.sol::verify`/`_verifySecp256k1`. atakit side: stubs.rs
  `AlgoId`/`PublicIdentity`/`PublicIdentity::secp256k1`.
- **Found:** `_ECDSA_KEY_LENGTH = 65` (GuardianModule.sol:37). secp256k1 key
  expected as 65-byte uncompressed `0x04 || x || y` (SignatureVerifier.sol:113-132
  requires `key.length == 65 && key[0] == 0x04`). `ALGO_ID_ES256K = 3`
  (Constants.sol:52) == atakit `AlgoId::Es256K = 3` (stubs.rs:48) == the guardian's
  hardcoded `typeId: 3`. atakit `PublicIdentity::secp256k1` produces exactly the
  65-byte uncompressed form (stubs.rs:60-70).
- **Conclusion:** RESOLVED. typeId=3/ES256K/secp256k1 and the 65-byte uncompressed
  key length agree across Rust guardian, atakit, and the `feat/tdx` contract.
  Field-5 (rotate pubKey) byte-matches. The `_ECDSA_KEY_LENGTH == 65` assumption
  in the table above is now confirmed, not conditional.

#### Q3 (W0): single-branch coverage of BOTH rotate + custody
- **Examined:** `git branch -a`, grep for `ROTATE_GUARDIAN_KEY` across branches.
- **Found:** `rotateGuardianKey` / `ROTATE_GUARDIAN_KEY` exists ONLY on
  `origin/feat/tdx`. The 7-field custody preimage exists ONLY on
  `origin/fix/increase-guardian-signatures-security`, which has NO rotate-key.
- **Conclusion:** Confirms the W0 dual-bind exactly as stated: no single branch
  byte-matches BOTH preimages. `feat/tdx` makes rotate-key match (this property)
  while custody drifts; `fix/...` is the inverse. Property statement UNCHANGED.

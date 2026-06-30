# keygen-evidence-accepted-onchain

Added during the evaluation pass (gap G1 — coverage-balance CW-1, wildcard
F-FIND-4). This is the **keygen twin of [[custody-preimage-matches-onchain-verifier]]**,
but unlike the custody attestation path (inert under precondition P1), this path
is **verified on-chain in production today**.

## What led to this property

`POST /eth/v1/keygen` returns `KeyGenResponse { pk_hex, evidence }` where
`evidence: AttestationEvidence { session_id, signature, session_public_key,
owner_public_key }` (`src/enclave/types.rs:11-32`, `src/io/remote_attestation.rs:14-43`).

reef consumes the **entire** response, not just the keccak digest:
`reef-guardian/src/handlers/api/guardian/rotate_guardian_key.rs:88-195` repackages
`pk_hex` + `evidence` into a `GuardianSessionProof` and submits it to
`GuardianModule.rotateGuardianKey(...)`, which verifies it **on-chain** via
`SessionRegistry`. This is a live, on-chain-verified production path (the guardian
key-rotation flow), distinct from the custody `verify_session` path that reef
currently disables.

`rotate-key-preimage-matches-onchain` only asserts the **keccak `signedMessageHash`**
(the `ROTATE_GUARDIAN_KEY` abi-encode preimage). It does **not** cover:

1. The `owner_public_key` / `session_public_key` **`PublicIdentity` round-trip**:
   the contract requires `typeId == ALGO_ID_ES256K (3)` and a 65-byte uncompressed
   secp256k1 key (`puffer-contracts feat/tdx` `SignatureVerifier.sol:113-132`,
   `Constants.sol:52`). The guardian copies these from the CVM agent's
   `PublicIdentity` (`remote_attestation.rs:37-41`) — any typeId/length/encoding
   drift makes `rotateGuardianKey` revert.
2. The `pk_hex` **uncompressed → `ecies::parse` round-trip**: keygen returns the
   uncompressed 65-byte pubkey (`types.rs:18-24`); reef/contract must parse it back
   to the same curve point (see [[pubkey-representation-consistent]]).

## What goes wrong if violated

A drift in the `PublicIdentity` encoding or the `pk_hex` round-trip makes the
guardian's freshly-generated, attested key **un-rotatable on-chain** — the
guardian cannot register its new key, so it cannot participate in custody/exit
signing. Because all guardians run the same binary, this is a correlated,
protocol-wide liveness failure (the keygen analogue of W0).

## Antithesis angle

Primarily a **cross-process agreement / regression** property (like the other
preimage properties — see the antithesis-fit caveat): exercised by generating
diverse keygen inputs (block_number, chain_id, guardian_module_address) and
asserting the full `GuardianSessionProof` is accepted by `rotateGuardianKey`.
- **Primary form (cheap, no chain):** a Rust/`alloy sol!` reference encoder +
  `PublicIdentity` shape check, differentially compared to what the guardian emits
  — catches encoding drift deterministically.
- **Secondary form (anvil):** submit `rotateGuardianKey` to a minimal
  GuardianModule + SessionRegistry deploy and assert acceptance.
Faults add little here beyond the CVM-agent `hang`/`error`/`empty` modes (which
overlap [[cvm-stub-never-in-production]] and [[dependency-hang-makes-progress]]).

## Suggested instrumentation

- `Always("rotateGuardianKey accepts the guardian's keygen evidence")` against a
  reference encoder / anvil. **MISSING.**
- `Always("keygen evidence session/owner PublicIdentity has typeId==3 and a 65-byte
  key")` — a cheap shape assertion at the keygen response boundary. **MISSING.**
- `Sometimes("keygen evidence carried a non-default (real) attestation")` to
  confirm the workload exercises the non-stub path. **MISSING.**

## Open Questions

- Which deployed `GuardianModule` bytecode does `rotateGuardianKey` run? Shares
  catalog Q1 / the W0 deployment question — mechanically verifiable (see
  [[custody-preimage-matches-onchain-verifier]]). `(needs human input / cast call)`
- Does the **real** DCAP CVM agent emit `PublicIdentity` with `typeId==3` and a
  65-byte key, matching the contract? `(partial: the sim does; real agent
  closed-source — needs human input.)`
- Is key rotation a frequent or rare operation? `(needs human input — sets how
  often this live path actually runs in production.)`

### Investigation Log

This property was added in the evaluation pass and reuses confirmed evidence; its
open questions were investigated under sibling properties rather than re-run here.

#### Which deployed GuardianModule bytecode runs rotateGuardianKey?
- Shares the W0 deployment question — see the Investigation Log in
  [[custody-preimage-matches-onchain-verifier]] (Foundry broadcast artifact
  indicates a 5-field GuardianModule deployed to Hoodi 2026-03-10; reef pins the
  addresses). Conclusion: mechanically verifiable via `cast call`/`eth_getCode`;
  remaining unknown is proxy-upgrade status. Tagged `(needs human input)`.

#### Does the real DCAP agent emit PublicIdentity{typeId:3, 65-byte}?
- See the Investigation Log in [[rotate-key-preimage-matches-onchain]] and
  [[session-public-key-parse-no-wrong-identity]]: the atakit **sim** emits the
  ES256K (typeId 3) / 65-byte shape and the contract `SignatureVerifier` fails
  closed on a mismatch; the production DCAP binary is closed-source. Tagged
  `(partial)`: sim confirmed, real agent needs human input.

# atakit migration decisions

## Current configuration (2026-09-23)

The current production TOMLs target format 8 and `automata-linux:v0.3.1-debug`,
with new workload version `v0.2.1`. Emulator now references these same production
TOMLs. TPM policies remain in the files and are ignored for local directory storage.
The earlier separate `*-local` definitions have been removed. See [container/README.md](../container/README.md) for current commands.
Previously built format-7 archives must be rebuilt, not reused. No cloud deploy
or public-chain publication is part of this update.

The entries below are historical validation records for the earlier migration,
not statements about the current artifacts or a successful production deployment.

## Original migration record

Recorded 2026-09-08. Workload packaging, the Rust portal adapter, registry ABI,
and local Puffer GuardianModule signing-domain alignment are implemented and
validated locally. Publishing and deployment are explicitly deferred by the
operator. No cloud resources or chain transactions were created.

## Confirmed target

- Network: Ethereum Hoodi, chain ID 560048.
- Platform: GCP Intel TDX.
- Base image: `automata-linux:v0.3.0-debug`.
- SessionRegistry: `0xafbdf3e5f515f126C09EE5f01CF2B4e0E426b13A`.
- WorkloadRegistry: `0xE4B77052e0F9839F95A9e49F531d568F17D260dB`.
- BaseImageRegistry: `0x8C5b11Ff975e325417aED3A61Cb37F513F4f27e3`.

These are Puffer's own deployment of automata-tee-workload-measurement at
`blocksec-20260909-final-audited` (`0e0991b`), the commit Puffer's audited
contracts pin. Automata's newer Hoodi deployment (SessionRegistry
`0x7575BceC155b272077C87aD3Ae5Ef54Cf9DC6601`) was deployed before the BlockSec fixes and is
superseded for Puffer, as is the earlier selection
`0xB247950fBBFCE245641e433AFd7d8884328CE5A1`.
Configure these chain settings in the atakit operator configuration alongside
the GCP provider/target and owner/gas keys.

The examples identify this publisher-qualified base image:
`0xaef8fc89416f01494ec6534de68d30aab26d7598db8a05967b0ba7d3ecb259d2/automata-linux:v0.3.0-debug`.
They record base image ID
`0x2fdcb9f3ffdcaaf693739c5bdec264f8f5d2404d7a268bf407e62ef9a2439445`.
These image details come from example documentation, not an independent live
image-policy verification in this investigation.

## Sources and verification boundary

Local checkouts inspected:

- atakit-ng: `/Users/cheney/ata/cvm/atakit-ng`, updated to commit `8853ffc`
  (workspace version 0.6.0, format 7). The operator replaced the installed CLI;
  `/Users/cheney/.cargo/bin/atakit` reports 0.6.0.
- atakit-portal: `/Users/cheney/ata/cvm/atakit-portal`, commit `079cb59`.
- Contracts: `/Users/cheney/ata/tee-workload-attestation`, commit `0e0991b`.
- Examples: `/Users/cheney/ata/cvm/cvm-workload-examples`.

Hoodi read-only RPC calls confirmed the selected SessionRegistry's associated
WorkloadRegistry and BaseImageRegistry above, plus:

- Proxy implementation: `0xC42FC8b09215Dd83d50e7F57a6a3fCfeef88c75C`.
- TeeSecurityPolicyVerifier: `0x64d721a7CED09E5C71aaFE9041c5647D93A9640C`.
- TpmVerifier: `0xAfbE49aF1fD1F9a17Bc7Bb66CE7688B8f3bf5444`.

The implementation bytecode contains the current registration (`0x5605921d`),
renewal (`0xd395b2c7`), recovery (`0x114d91fe`), and signature-verification
(`0xc243425e`) selectors. Portal selectors were calculated from its actual ABI
types; its comment naming the old `0xd5269dcc` registration selector is stale.
This is interface evidence, not proof that the deployment exactly matches the
contract checkout or that a full TDX session can register successfully.

## Application interface

Enable `atakit-portal = true` in each workload that requests signatures. The
framework mounts a service-specific socket at `/run/atakit-portal.sock`.
Use HTTP/JSON over this Unix socket, not the supervisor-private IPC client.

- `POST /sign-message`: send hex message bytes, `hash_fn = "keccak256"`, and
  optionally a canonical lowercase `expected_session_id`.
- Verify the digest as
  `keccak256(UTF8("ATAKIT_SESSION_SIGN_V1") || message_bytes)`.
- Parse `session_id`, `session_pubkey`, `signature`, `message_hash`, and
  `hash_fn` from the response. The response does not contain an owner public key.
- A session-not-ready response is HTTP 503 with `Retry-After: 1` and
  `code = "session_not_ready"`. Retry with bounds and a timeout.
- HTTP 409 `session_id_mismatch` means the committed session differs from the
  expected session; refresh session evidence before retrying a pinned request.
- The workload socket exposes `GET /portal-external-api/status` and
  `GET /portal-external-api/session/evidence-bundle?challenge=...`.
  Do not assume the external `GET /session` route is mounted on the workload
  socket: the production router in the inspected checkout does not mount it.

Sessions can change within one VM boot. With required registration, containers
start before the initial session is built; workload ports open after chain
registration is verified. A container process starting is not proof of session
readiness.

## Implemented and verified locally

- Replaced the root `atakit.json` with three format-7 workload manifests and
  native `atakit workload build -d . --file container/<service>/atakit-workload.toml`
  commands. `--file` is a local atakit extension; unmodified 0.6.0 lacks it.
  The former source-copying build script has been removed.
- Configured the selected base-image whitelist, Hoodi genesis fork version,
  new SessionRegistry, portal socket, and separate encrypted `/data` disks.
- Replaced the `automata-cvm-agent` git dependency with a workload-facing HTTP
  client. The client checks the domain-separated digest, ES256K key fingerprint,
  and recoverable signature; it supports session pinning, at most three readiness
  attempts, a 15-second total timeout, and a 64 KiB response limit. It returns
  409 and unrelated failures to the caller without switching session identity.
- Corrected `CVMSession` to the registry's seven bytes32 and two uint64 fields.
  Owner and active status are separate queries. Custody verification still calls
  the registry and retains its expected-workload check.
- Aligned `../puffer-contracts/mainnet-contracts/src/GuardianModule.sol` with
  `ATAKIT_SESSION_SIGN_V1` while preserving its owner and workload checks.
  The application still sends the original ABI-encoded rotation payload; callers
  do not prepend the domain. No contract deployment was performed.
- Added Cargo.lock as a versioned build input and pinned ECIES to 0.2.10, the final
  0.2.x release using the libsecp256k1 key API required by the application.
- The initial amd64-emulated Rust compiler crashed even on `rustc -vV`.
  A temporary cross-compilation workaround succeeded on the operator's Apple
  Silicon Docker host. It has since been removed and the original Dockerfile
  restored. Buildx's target platform selects the compiler and runtime architecture.
  The earlier successful build does not validate this revised build path.

Validation performed:

- Nine Rust tests: eight portal wire/validation tests and one registry ABI test.
  The legacy client first failed on the new JSON response and the legacy ABI
  failed to decode the current registry tuple; both now pass their regression tests.
- Three focused Solidity tests pass against real ECDSA verification: accept a
  portal key-rotation signature, reject an old undomained signature, and preserve
  workload policy rejection. All three failed before the domain change.
  The rotation vector is also accepted by the Rust portal adapter.
- The real `atakit-portal-state` parser and all its manifest validation methods
  accept each generated manifest.
- Inspected all three archives: format 7, measured publisher, image archive hashes,
  `linux/amd64` runtime, direct binary entrypoint, and persistent storage mapping.
- Loaded each generated image into local Docker, started it with its measured
  environment and a temporary `/data`, and received HTTP 200 from `/upcheck`.
  Containers were stopped afterward; this does not establish TDX attestation.
- Nineteen existing cryptographic tests pass, including ECIES envelope encryption,
  BLS shares, signatures, and keystore import/export. They ran in a temporary
  working directory so the operator's keys and slashing data were not touched.
- `cargo check --locked --all-targets` passes with existing unused-code warnings.
  The library and all three workload binaries also pass their focused check.

Focused verification commands:

```bash
# validator-guardian
cargo test --locked --test portal_client --test session_registry_abi
cargo check --locked --lib --bin guardian --bin validator --bin secure-signer

# puffer-contracts/mainnet-contracts: isolate these tests from the older
# workload-verifier test suite, which still uses the pre-session API.
FOUNDRY_SRC=test/portal FOUNDRY_TEST=test/portal FOUNDRY_SCRIPT=test/portal \
  forge test --match-contract GuardianModulePortalTest
```

The optional `client` executable target is pre-existing incomplete configuration:
`src/client/mod.rs` is a library module without `main` and cannot compile as an
executable with `--all-features`. That unrelated target has not been redesigned.

The generated archives under `build/workloads/` are **local validation artifacts**.
The initial builds used a temporary publisher; subsequent builds may use the
configured owner. Inspect each archive's publisher before release. No workload
has been published or deployed by this migration.
`atakit workload info` may display an unknown workload ID without a configured
publisher; `meta.publisher` in all three actual manifests was inspected directly.

## Native build entry update (2026-09-09)

The local atakit CLI now accepts `workload build -d <root> --file <config>`.
Relative config paths resolve under `-d`; image and package data paths continue
using that root. Default config selection and path-boundary checks are unchanged.
The old source-copying `scripts/build-workload.sh` was removed. All three configs
use the repository root as their build context, filtered by `.dockerignore`.

The atakit workload suite passes 205 tests, including config selection with
relative and absolute paths, root-relative resources, and missing-config errors.
The release CLI was compiled and installed at `/Users/cheney/.cargo/bin/atakit`.
Its previous binary is backed up at
`build/atakit-before-file-chzuwp9d/atakit`. The extension is a local source change,
not an upstream release feature.

All three native CLI builds succeeded with `--engine docker --no-store`; logs
are at `build/<service>-native-build.log`. Validator's archive SHA-256 remains
`685679893d3e45765ef9101c8f2eba350bead3ff7b65a4d4d64515cb3e6c1b46`, identical to
the preceding staging-script build using the same owner. Deployment remains deferred.

## Proof response compatibility

`AttestationEvidence` now contains `session_id`, `signature`, and
`session_public_key`. The latter remains a public identity (`typeId`, `key`).
The portal has no owner public key in its signing response, so the application
no longer emits `owner_public_key`. A proof submitter supplies its deployment
owner's public identity separately when constructing `GuardianSessionProof`;
GuardianModule checks that fingerprint against `getSessionOwner`. Do not replace
it with the session public key or an empty placeholder. External reef/coral proof
submitters have not been changed in this workspace.

## Deferred deployment verification

Deployment is out of the current scope at the operator's request. A later run
will need the intended publisher/owner/gas key configuration, GCP target, and
fresh-disk versus data-recovery decision. The archives must be rebuilt for that
publisher before workload policy publication.

Real GCP TDX session registration, TPM disk unlocking/restart persistence, old
key/slashing-data recovery, and end-to-end keygen/custody/rotation/exit signing
remain unverified. Local wire tests and image health checks do not replace them.

# Migration Guide: SGX/IAS → TDX/atakit

This guide is for consumers of the secure-signer, validator, and guardian services who need to update their integrations after the migration from Intel SGX (EPID attestation via Intel IAS) to Intel TDX (session-based attestation via Automata atakit).

**Base commit:** `e016932` (last SGX release)
**Target commit:** latest on `AB#4275/update-to-tdx`

---

## Quick Summary

| What changed | Old (SGX) | New (TDX) |
|---|---|---|
| TEE platform | Intel SGX inside Occlum LibOS | Intel TDX Confidential VM |
| Attestation model | Per-request EPID quote → Intel IAS report | Per-request CVM Agent session signature (session registered on-chain via Automata SessionRegistry) |
| Identity anchor | `MRENCLAVE` + `MRSIGNER` (SGX measurements) | `workload_id` (atakit-measured Docker image hash) |
| Attestation verification | Parse Intel x509 cert chain + EPID quote locally (using `openssl`) | Call `SessionRegistry.verifySessionSignature()` on-chain |
| `AttestationEvidence` fields | `raw_report`, `signed_report`, `signing_cert` | `session_id`, `signature`, `session_public_key`, `owner_public_key` |
| `BlsKeygenPayload` fields | `intel_report`, `intel_sig`, `intel_x509` | `session_id`, `attestation_signature`, `session_public_key` |
| `ValidateCustodyRequest` fields | `mrenclave`, `mrsigner`, `verify_remote_attestation` | `workload_id`, `verify_session` |
| Cloud platform | Azure DC-Series (SGX enclaves) | GCP c3-standard-4 (TDX-capable) |
| Container runtime | Ubuntu 20.04 + Occlum + SGX runtime packages | Debian Bookworm-slim, standard Docker |
| Container image build | Per-service Dockerfile + `build_image.sh` script with Occlum packaging | Single multi-stage `container/Dockerfile` parameterized with `BINARY_NAME` |
| Starting the service | `occlum run /bin/secure-signer <port>` | `docker compose up` or binary directly |
| Key storage path | `./etc/keys/` (Occlum FS) | `./data/keys/` (encrypted disk volume) |
| Port configuration | CLI argument only | Env var > CLI arg > default |
| Rust toolchain | Not pinned | Pinned to Rust 1.91 (`rust-toolchain.toml`) |
| Build target | `x86_64-unknown-linux-musl` (static linking for Occlum) | Default target (standard `glibc`) |
| `sgx` feature flag | `[features] sgx = []` controlled attestation behavior at compile time | Removed — runtime `CVM_AGENT_STUB=true` env var replaces `#[cfg(not(feature = "sgx"))]` |
| `openssl` crate dependency | Required for Intel cert chain verification | Removed — no longer needed |
| CVM Agent communication | FFI call to C++ EPID RA library (`do_epid_ra`) | Unix socket call to CVM Agent (`/app/cvm-agent.sock`) via `automata-cvm-agent` crate |
| `AttestationEvidence::new()` | Synchronous | **Async** (`async fn new()`) |

---

## Breaking API Changes

### 1. `AttestationEvidence` — returned in `KeyGenResponse`

This type is embedded in the response to all four keygen endpoints:
- `POST /eth/v1/keygen/secp256k1` (secure-signer)
- `POST /eth/v1/keygen/bls` (secure-signer)
- `POST /eth/v1/keygen` (guardian)
- `POST /bls/v1/keygen` (validator) — returned inside `BlsKeygenPayload`

**Old shape (3 fields):**

```json
{
  "pk_hex": "0x04...",
  "evidence": {
    "raw_report": "{\"id\":\"...\",\"timestamp\":\"...\",\"isvEnclaveQuoteStatus\":\"SW_HARDENING_NEEDED\",\"isvEnclaveQuoteBody\":\"...\"}",
    "signed_report": "<base64 RSA signature>",
    "signing_cert": "-----BEGIN CERTIFICATE-----\n...\n-----END CERTIFICATE-----\n"
  }
}
```

**New shape (4 fields):**

```json
{
  "pk_hex": "0x04...",
  "evidence": {
    "session_id": "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef",
    "signature": "0xabcdef...",
    "session_public_key": { "key": "0x04...", "key_type": "secp256k1" },
    "owner_public_key": { "key": "0x04...", "key_type": "secp256k1" }
  }
}
```

**Field mapping:**

| Old field | New field | Notes |
|---|---|---|
| `raw_report` | `session_id` | B256 hex — references the on-chain session |
| `signed_report` | `signature` | Hex bytes — CVM Agent's session key signature over the attested payload |
| `signing_cert` | `session_public_key` | `PublicIdentity` object — the session key that produced the signature |
| *(none)* | `owner_public_key` | **New field** — `PublicIdentity` object identifying the CVM Agent owner |

> **Note:** `session_public_key` and `owner_public_key` are `PublicIdentity` structs from the `automata-cvm-agent` crate, not plain hex strings. The exact serialization depends on the `automata-cvm-agent` version.

**Migration:** Replace any code that reads `raw_report`, `signed_report`, or `signing_cert` from the `evidence` object with the new fields. Add handling for the new `owner_public_key` field.

---

### 2. `BlsKeygenPayload` — returned by `POST /bls/v1/keygen` (validator)

The `BlsKeygenPayload` struct carries attestation evidence fields inline (not nested inside an `AttestationEvidence` object). These three fields were renamed:

**Old fields (removed):**

```json
{
  "bls_pub_key_set": "...",
  "bls_pub_key": "...",
  "signature": "...",
  "deposit_data_root": "...",
  "bls_enc_priv_key_shares": ["...", "..."],
  "intel_report": "{ IAS JSON body }",
  "intel_sig": "<base64 RSA signature>",
  "intel_x509": "-----BEGIN CERTIFICATE-----\n...",
  "guardian_eth_pub_keys": ["0x04...", "0x04..."],
  "withdrawal_credentials": "...",
  "fork_version": [0, 0, 0, 0]
}
```

**New fields:**

```json
{
  "bls_pub_key_set": "...",
  "bls_pub_key": "...",
  "signature": "...",
  "deposit_data_root": "...",
  "bls_enc_priv_key_shares": ["...", "..."],
  "session_id": "0x1234...",
  "attestation_signature": "0xabcd...",
  "session_public_key": "0x04ef...",
  "guardian_eth_pub_keys": ["0x04...", "0x04..."],
  "withdrawal_credentials": "...",
  "fork_version": [0, 0, 0, 0]
}
```

**Field mapping:**

| Old field | New field |
|---|---|
| `intel_report` | `session_id` |
| `intel_sig` | `attestation_signature` |
| `intel_x509` | `session_public_key` |

All other fields (`bls_pub_key_set`, `bls_pub_key`, `signature`, `deposit_data_root`, `bls_enc_priv_key_shares`, `guardian_eth_pub_keys`, `withdrawal_credentials`, `fork_version`) are **unchanged**.

---

### 3. `ValidateCustodyRequest` — sent to `POST /guardian/v1/validate-custody`

**Old shape:**

```json
{
  "keygen_payload": { ... },
  "guardian_enclave_public_key": "0x04...",
  "mrenclave": "0xdd4678fd...",
  "mrsigner": "0xabcdef12...",
  "verify_remote_attestation": true,
  "validator_index": 42
}
```

**New shape:**

```json
{
  "keygen_payload": { ... },
  "guardian_enclave_public_key": "0x04...",
  "workload_id": "<atakit workload hash>",
  "verify_session": true,
  "validator_index": 42
}
```

**Field mapping:**

| Old field(s) | New field | Notes |
|---|---|---|
| `mrenclave` + `mrsigner` | `workload_id` | Single field replaces two. The workload ID is the atakit-computed hash of the Docker image + measured config — the TDX equivalent of MRENCLAVE |
| `verify_remote_attestation` | `verify_session` | Same boolean semantics — controls whether session evidence is validated |

---

### 4. Removed: `KeyGenResponse::validate_eth_ra()` and `validate_bls_ra()`

The old `KeyGenResponse` type had two methods for client-side attestation verification:
- `validate_eth_ra(expected_mrenclave: &String) -> Result<EthPublicKey>`
- `validate_bls_ra(expected_mrenclave: &String) -> Result<BlsPublicKey>`

Both are **removed**. They relied on `openssl` for Intel cert chain parsing and EPID quote parsing, which no longer applies.

**Migration:** Replace calls to these methods with on-chain `SessionRegistry.verifySessionSignature()` verification (see [How to Verify Attestation](#how-to-verify-attestation-new) below).

---

### 5. Guardian route fix — `POST /eth/v1/keygen` now works correctly

In the old SGX codebase, the guardian binary registered `POST /eth/v1/keygen` and `GET /eth/v1/keygen` as two separate `.route()` calls on the same path. Axum silently dropped the first registration (POST), making the ETH keygen endpoint unreachable via POST.

This is fixed — `POST` and `GET` are now registered on the same `.route()` call using method chaining.

**Action required:** If your client was working around this bug (e.g., calling a different URL, or handling `405 Method Not Allowed`), remove the workaround.

---

## Unaffected Endpoints

The following endpoints have **no breaking changes** — their request and response shapes are identical:

| Endpoint | Service | Notes |
|---|---|---|
| `GET /upcheck` | all | Health check, unchanged |
| `GET /eth/v1/keystores` | secure-signer, validator | List BLS keys, unchanged |
| `GET /eth/v1/keygen/secp256k1` | secure-signer | List ETH keys, unchanged |
| `GET /api/v1/eth2/publicKeys` | validator | List BLS pubkeys for VC, unchanged |
| `POST /api/v1/eth2/sign/:bls_pk_hex` | secure-signer, validator | BLS signing, unchanged |
| `POST /api/v1/eth2/deposit` | secure-signer | Deposit signing, unchanged |
| `POST /guardian/v1/sign-exit` | guardian | Voluntary exit signing, unchanged |

---

## How to Verify Attestation (New)

### Old: Client-side Intel IAS verification

Previously, clients called `KeyGenResponse::validate_eth_ra(expected_mrenclave)` or `validate_bls_ra(expected_mrenclave)` which:

1. Parsed the PEM cert chain from `signing_cert`
2. Verified the CN of the signing cert was `"Intel SGX Attestation Report Signing"`
3. Verified the CN of the root CA was `"Intel SGX Attestation Report Signing CA"`
4. Verified the chain using an OpenSSL X.509 trust store
5. Base64-decoded `isvEnclaveQuoteBody` from the `raw_report` JSON
6. Parsed the 432-byte EPID quote binary
7. Extracted `MRENCLAVE` at bytes [112..144] and compared to the expected value
8. Extracted `REPORTDATA` at bytes [368..432] and verified the public key was embedded

### New: On-chain SessionRegistry verification

Attestation verification is now done on-chain. The `session_id` in the keygen response refers to a session that was registered by a genuine TDX CVM Agent at boot via Automata's `SessionRegistry` contract.

**Verification steps:**

1. Extract `session_id`, `signature`, and the message (the public key bytes) from the keygen response
2. Call `SessionRegistry.verifySessionSignature(session_id, message, signature)` on-chain
3. The contract confirms:
   - The `session_id` was registered by a genuine CVM Agent backed by a valid TDX DCAP quote
   - The `signature` over `message` was produced by the session key for that `session_id`
4. Optionally verify `owner_public_key` matches the expected operator

**What `workload_id` replaces:** MRENCLAVE identified which specific enclave binary was running. The `workload_id` in `ValidateCustodyRequest` is the atakit workload hash — it identifies which Docker image + measured config was deployed. Obtain it via `atakit build-workload <name>` after building and before deploying.

> **Note:** The guardian's `verify_session_evidence()` currently performs structural validation only (checks that `session_id`, `attestation_signature`, and `session_public_key` are non-empty, and reconstructs the attestation payload). Full on-chain verification via `SessionRegistry.verifySessionSignature()` is deferred to the caller.

---

## Infrastructure and Deployment Changes

### Cloud platform

| | Old | New |
|---|---|---|
| Cloud | Azure DC-Series VMs (SGX enclaves) | GCP c3-standard-4 (or any TDX-capable host) |
| SGX devices | `/dev/sgx/enclave`, `/dev/sgx/provision` mounted into container | Not required — service runs as a standard process inside the TDX CVM |
| AESMD service | `/var/run/aesmd` socket mounted | Not required |
| CVM Agent | N/A | Unix socket at `/app/cvm-agent.sock` (mounted into container, managed by atakit) |

### Starting the service

**Old (inside Occlum container):**

```bash
# Build with SGX support
cargo build --release --features sgx --target x86_64-unknown-linux-musl
# Package with Occlum, then run inside the container
occlum run /bin/secure-signer 9001
```

**New (plain Docker):**

```bash
# Via docker compose (recommended for atakit deployments)
docker compose -f container/secure-signer/docker-compose.yml up
docker compose -f container/validator/docker-compose.yml up
docker compose -f container/guardian/docker-compose.yml up

# Or run the binary directly (local dev)
cargo run --bin secure-signer   # uses SECURE_SIGNER_PORT env var or defaults to 9001
cargo run --bin validator        # uses VALIDATOR_PORT env var or defaults to 3031
cargo run --bin guardian         # uses GUARDIAN_PORT env var or defaults to 3031
```

### Port configuration

All three binaries now read their port from an environment variable first, falling back to the CLI argument, then the built-in default:

| Binary | Env var | CLI arg position | Default |
|---|---|---|---|
| `secure-signer` | `SECURE_SIGNER_PORT` | 1st arg | `9001` |
| `validator` | `VALIDATOR_PORT` | 1st arg | `3031` |
| `guardian` | `GUARDIAN_PORT` | 1st arg | `3031` |

`GENESIS_FORK_VERSION` can also be set via env var for all three binaries (hex string, e.g., `00000000`).

### Docker images

**Old:** Based on `ubuntu:20.04`. Required SGX/Occlum runtime packages:
```
libsgx-epid  libsgx-quote-ex  libsgx-dcap-ql  libsgx-urts
libsgx-uae-service  libsgx-dcap-default-qpl  occlum-runtime
```

Built via custom `container/build_image.sh` script with Occlum packaging.

**New:** Based on `debian:bookworm-slim`. Only requires:
```
ca-certificates  libssl3
```

All three services build from a single multi-stage `container/Dockerfile` using `ARG BINARY_NAME`. Docker Compose files are provided per-service under `container/<service>/docker-compose.yml`.

### CVM Agent socket

Each container must mount the CVM Agent Unix socket. The docker-compose files configure this:

```yaml
volumes:
  - ./cvm-agent.sock:/app/cvm-agent.sock
```

The socket path can be overridden via `CVM_AGENT_SOCKET_PATH` env var (default: `/app/cvm-agent.sock`).

### Key storage path

Keys are no longer stored inside the Occlum filesystem. They are stored on the encrypted data disk volume mounted at `/data`:

| | Old path | New path |
|---|---|---|
| BLS keys | `./etc/keys/bls_keys/` | `./data/keys/bls_keys/` |
| ETH keys | `./etc/keys/eth_keys/` | `./data/keys/eth_keys/` |
| Slashing DB | `./etc/slashing/` | `./data/slashing/` |

In Docker, `/data` is mounted as a named volume (configured as a 10 GB encrypted disk in `atakit.json`). Keys persist across container restarts.

**Action required:** If you are migrating an existing deployment with keys in `./etc/keys/`, copy the key files to the new paths before starting the new service.

### Network configuration

Each service has a `network_config.json` file mounted at `/config/network_config.json` containing fork info and network details:

```json
{
    "network_name": "ephemery",
    "deposit_cli_version": "2.3.0",
    "fork_info": {
        "fork": {
            "previous_version": "0x1000101a",
            "current_version": "0x1000101b",
            "epoch": "0"
        },
        "genesis_validators_root": "0x270d43e74ce340de4bca2b1936beca0f4f5408d9e78aec4850920baf659d5b69"
    }
}
```

Update these values for your target network (mainnet, holesky, etc.) before deploying.

---

## Deploying with atakit

The project now includes `atakit.json` at the repo root, defining workloads for all three services.

### Build a workload

```bash
# Build one workload
atakit build-workload secure-signer
atakit build-workload validator
atakit build-workload guardian
```

This produces a measured artifact. The hash of this artifact is your `workload_id`.

### Deploy to GCP

```bash
atakit deploy secure-signer-tdx
atakit deploy validator-tdx
atakit deploy guardian-tdx
```

Each deployment runs on a separate `c3-standard-4` GCP instance (Intel TDX-capable) with a 10 GB encrypted data disk.

### Measured vs unmeasured configuration

atakit divides config into two categories:

| File | Purpose | Included in workload hash? |
|---|---|---|
| `container/secure-signer/env` | `RUST_LOG`, `SECURE_SIGNER_PORT` | Yes — changes invalidate the workload identity |
| `container/validator/env` | `RUST_LOG`, `VALIDATOR_PORT` | Yes |
| `container/guardian/env` | `RUST_LOG`, `GUARDIAN_PORT` | Yes |

### Docker Compose port mapping

The default docker-compose files expose services on different host ports to avoid conflicts when running multiple services on the same host:

| Service | Container port | Host port |
|---|---|---|
| secure-signer | 9001 | 9003 |
| validator | 9001 | 9002 |
| guardian | 9001 | 9001 |

---

## Build Changes (Library Consumers)

If you depend on `puffersecuresigner` as a Rust library:

### Removed dependencies
- `openssl` — no longer needed (Intel cert chain verification removed)
- `libc` — no longer needed (FFI to C++ RA library removed)
- `cc` build dependency — no longer needed (no C++ compilation)

### Added dependencies
- `automata-cvm-agent` — CVM Agent client from `https://github.com/automata-network/atakit` (branch `dev`)

### Removed feature flags
- `[features] sgx = []` is removed. The old `#[cfg(feature = "sgx")]` / `#[cfg(not(feature = "sgx"))]` compile-time branching is replaced by a runtime check: `CVM_AGENT_STUB=true` env var.

### Removed build target
- `[build] target = "x86_64-unknown-linux-musl"` is removed from `Cargo.toml`. Builds now use the default target.

### Rust toolchain
- A `rust-toolchain.toml` file now pins the toolchain to **Rust 1.91**.

### Async changes
- `AttestationEvidence::new()` is now **async** (it calls the CVM Agent over a Unix socket). Callers must `.await` it.
- `guardian::attest_new_eth_key_with_block_info()` is now **async** for the same reason.

### Removed files
- `CMakeLists.txt`, `build_epid_ra.sh` — C++ EPID RA library build infrastructure
- `lib/` directory (all C++ headers and sources for RA)
- `enclave.py` — Occlum enclave packaging script
- `src/io/ra_config.h`, `src/io/ra_wrapper.cpp` — C++ FFI wrappers
- `conf/` directory — old YAML config files (`guardian-rust-config.yaml`, `secure-signer-rust-config.yaml`, `validator-rust-config.yaml`, `ra_config.json`)

---

## Local Development and Testing

### Testing without TDX hardware

Set `CVM_AGENT_STUB=true` to bypass the CVM Agent call. `AttestationEvidence::new()` returns default empty evidence (all fields set to defaults), identical in purpose to how the old `#[cfg(not(feature = "sgx"))]` stub worked.

```bash
CVM_AGENT_STUB=true cargo test -- --test-threads 1
```

### Testing with a simulated CVM Agent

Use atakit's `sim-agent` to run a local mock CVM Agent that behaves like the real one (real session keys, real signatures — just not backed by TDX hardware):

```bash
# Terminal 1
atakit sim-agent   # starts mock CVM Agent

# Terminal 2
cargo run --bin secure-signer   # or validator / guardian

# Verify keygen returns real session evidence
curl -X POST http://localhost:9001/eth/v1/keygen/secp256k1
```

The `CVM_AGENT_SOCKET_PATH` env var lets you point at a different agent socket path (default: `/app/cvm-agent.sock`).

---

## Migration Checklist

### API consumers

- [ ] **`AttestationEvidence` fields**: Replace reads of `raw_report`, `signed_report`, `signing_cert` with `session_id`, `signature`, `session_public_key`, `owner_public_key`
- [ ] **`BlsKeygenPayload` fields**: Rename `intel_report` → `session_id`, `intel_sig` → `attestation_signature`, `intel_x509` → `session_public_key`
- [ ] **`ValidateCustodyRequest` fields**: Replace `mrenclave` + `mrsigner` with single `workload_id`; rename `verify_remote_attestation` → `verify_session`
- [ ] **Attestation verification**: Replace local Intel cert chain + MRENCLAVE comparison with on-chain `SessionRegistry.verifySessionSignature()` call
- [ ] **Remove `validate_eth_ra()` / `validate_bls_ra()` calls**: These methods no longer exist on `KeyGenResponse`
- [ ] **Guardian `POST /eth/v1/keygen`**: Remove any workarounds for the old duplicate route bug — this endpoint now works correctly

### Infrastructure / operations

- [ ] **Cloud VMs**: Provision TDX-capable VMs (GCP c3-standard-4 or equivalent) instead of Azure DC-Series SGX VMs
- [ ] **Container startup**: Remove `--device /dev/sgx/enclave`, `--device /dev/sgx/provision`, `-v /var/run/aesmd` mounts — they are no longer needed
- [ ] **CVM Agent socket**: Ensure `cvm-agent.sock` is mounted at `/app/cvm-agent.sock` (or set `CVM_AGENT_SOCKET_PATH`)
- [ ] **Port config**: Update deployment scripts to use `SECURE_SIGNER_PORT` / `VALIDATOR_PORT` / `GUARDIAN_PORT` env vars (CLI args still work as fallback)
- [ ] **Key migration**: If migrating a live deployment, copy key files from `./etc/keys/` → `./data/keys/` before cutover
- [ ] **Network config**: Create/update `network_config.json` for your target network and mount at `/config/network_config.json`

### Library consumers (Rust crate)

- [ ] **Remove `--features sgx`** from build commands — the feature flag no longer exists
- [ ] **Remove `--target x86_64-unknown-linux-musl`** unless you specifically need musl
- [ ] **Update `.await` calls**: `AttestationEvidence::new()` and `attest_new_eth_key_with_block_info()` are now async
- [ ] **Remove `openssl` dependency** if you only used it for SGX attestation verification
- [ ] **Install Rust 1.91+** to match the project's `rust-toolchain.toml`
- [ ] **`--mrenclave` flag**: No longer meaningful in the client CLI — use `workload_id` from atakit build output instead

# Validator Guardian

Validator Guardian (formerly Secure-Signer) provides remote signing and key management for Ethereum PoS validators, designed to run inside TDX Confidential VMs on GCP.

## Binaries

### `guardian`

The guardian binary runs the guardian enclave RPC server. It is responsible for:

- Generating attested ETH keys and listing them (`/eth/v1/keygen`)
- Validating custody of BLS key shares (`/guardian/v1/validate-custody`), including verifying the validator enclave's CVM session evidence on-chain via `SessionRegistry`
- Signing voluntary exit messages with the guardian's BLS key share (`/guardian/v1/sign-exit`)

**Used by**: [Reef](https://github.com/PufferFinance/reef) (reef-guardian) via the `GuardianClientTrait` HTTP client.

**Deployment**: Must be deployed to GCP with TDX for attestation to work. The atakit portal handles TDX DCAP attestation and signs payloads with the session key.

### `validator`

The validator binary runs the validator enclave RPC server. It is responsible for:

- Generating attested BLS keys with threshold secret sharing (`/bls/v1/keygen`)
- Listing BLS keystores (`/eth/v1/keystores`)
- General-purpose BLS signing following the [Web3Signer](https://consensys.github.io/web3signer/web3signer-eth2.html) spec (`/api/v1/eth2/sign/:bls_pk_hex`) — compatible with consensus clients

**Used by**: [Coral](https://github.com/PufferFinance/coral) (coral-cli) via the `ValidatorClientTrait` HTTP client, when an `enclave_url` is provided.

**Deployment**: This binary is for external users who want to run their own TDX infrastructure for additional trust guarantees. When no `enclave_url` is configured, coral-cli performs BLS key generation locally using the `generate_bls_keystore_handler` library function — no deployed validator binary is needed.

## TDX attestation

The atakit portal runs inside the TDX Confidential VM and communicates with the binaries over a Unix socket (`/run/atakit-portal.sock`). Not all endpoints use TDX — only those that need to produce or verify attestation evidence.

### guardian

| Endpoint | TDX usage |
| -------- | --------- |
| `POST /eth/v1/keygen` | **Produces attestation.** Generates an ETH key and calls `AttestationEvidence::new()` to sign the public key via the atakit portal. The returned evidence proves the key was generated inside a TDX enclave. |
| `POST /guardian/v1/validate-custody` | **Verifies attestation.** When `verify_session` is true, calls `verify_session_evidence()` which reconstructs the validator's attestation payload and verifies its signature on-chain via `SessionRegistry.verifySessionSignature()`. This confirms the BLS keygen payload came from a genuine TDX validator enclave. |
| `GET /eth/v1/keygen` | No TDX — lists existing ETH keys from disk. |
| `POST /guardian/v1/sign-exit` | No TDX — signs with the stored BLS key share. |
| `GET /upcheck` | No TDX — health check. |

### validator

| Endpoint | TDX usage |
| -------- | --------- |
| `POST /bls/v1/keygen` | **Produces attestation** when `do_remote_attestation` is true. Generates BLS keys with threshold secret sharing, then calls `AttestationEvidence::new()` to sign the keygen payload via the atakit portal. When false, returns default empty evidence. |
| `GET /eth/v1/keystores` | No TDX — lists existing BLS keys from disk. |
| `POST /api/v1/eth2/sign/:bls_pk_hex` | No TDX — BLS signing with stored key. |
| `GET /api/v1/eth2/publicKeys` | No TDX — lists BLS public keys. |
| `GET /upcheck` | No TDX — health check. |

## Library usage

Both reef and coral also depend on `puffersecuresigner` as a Rust library crate for:

- **HTTP client implementations** (`ClientBuilder`, `GuardianClient`, `ValidatorClient`) for calling the enclave RPCs
- **Types and crypto logic** (`BlsKeygenPayload`, `SignExitRequest`, `ValidateCustodyRequest`, etc.) — these include methods for BLS key parsing, deposit message verification, and key share decryption
- **Local key generation** (`generate_bls_keystore_handler`) — used by coral-cli in the non-enclave path
- **Eth2 types** (`Fork`, `ForkInfo`, `Root`, etc.)

## Development

Start with [From a fresh checkout to local development and Hoodi deployment](docs/local-development.md)
for installation, keys, Anvil, contract deployment, local debugging, and cloud deployment.

Use the current atakit master CLI (manifest format 8) with `workload build --file`
and Emulator support; see [container/README.md](container/README.md) for binary
selection and the matching Base Image.

### Local development

The portal client sends the original business payload over HTTP/JSON on the
workload Unix socket. It verifies the returned signing digest, session key
fingerprint, and signature. Callers do not add the portal signing domain themselves.

Run the focused wire-protocol and registry ABI tests without TDX hardware:

```bash
cargo test --locked --test portal_client --test session_registry_abi
```

For native or Compose development with emulated Portal sessions, use
[the Emulator workflow](container/README.md#local-emulator) (`just emulator-up`,
`just run guardian`, or `just compose guardian up --build`).
For the IDs to use in custody requests and contract allowlists, read
[workload identity and business allowlists](docs/local-development.md#workload-identity-and-business-allowlists).
Emulator preserves business authorization checks but simulates hardware evidence;
local success does not establish production code identity.

For a hands-on introduction, follow the [local testing guide](docs/testing.md):
Guardian keygen → Validator keygen → custody, with isolated test data and
copyable HTTP requests. The guide separates business testing from TEE validation.

Use Emulator for local attestation requests and a real Portal for CVM deployments.
Portal provides session signatures and owner evidence for the attestation flow.

## Deployment

Workloads target GCP TDX VMs (`c3-standard-4`) on Hoodi using
`automata-linux:v0.3.1-debug`. The Guardian and Validator definitions under `container/` replace
the old `atakit.json` and use separate encrypted 10 GB data disks.

See [Workload build and deployment](container/README.md) for the format-8 toolchain
requirement, configuration, and verification boundaries.

### Build and deploy

```bash
# Run from the repository root with the local atakit CLI supporting --file.
atakit workload build -d . --file container/guardian/atakit-workload.toml -o build/workloads
atakit workload build -d . --file container/validator/atakit-workload.toml -o build/workloads
```

### Configuration

Each `container/<service>/atakit-workload.toml` declares the measured environment,
Hoodi genesis fork version, portal socket access, ports, and persistent `/data`
storage. Configure Hoodi, image repositories, GCP targets, and signing keys in
your atakit operator configuration. Emulator generates a separate Compose file
for each workload; see [container/README.md](container/README.md) for usage.

### Port mapping

| Service | Container port | Host port | Env var |
| ------- | -------------- | --------- | ------- |
| guardian | 9001 | 9001 | `GUARDIAN_PORT` |
| validator | 9001 | 9002 | `VALIDATOR_PORT` |

All binaries read port from: env var > CLI arg > default.

## API

- [API Documentation](https://pufferfinance.github.io/secure-signer-api-docs/redoc-static.html)

## Acknowledgements

Secure-Signer was funded via an [Ethereum Foundation grant](https://blog.ethereum.org/2023/02/22/allocation-update-q4-22).

## License

Released under Apache 2.0 License. See [LICENSE](LICENSE).

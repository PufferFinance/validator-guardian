# Validator Guardian

Validator Guardian (formerly Secure-Signer) provides remote signing and key management for Ethereum PoS validators, designed to run inside TDX Confidential VMs on GCP.

## Binaries

### `guardian`

The guardian binary runs the guardian enclave RPC server. It is responsible for:

- Generating attested ETH keys and listing them (`/eth/v1/keygen`)
- Validating custody of BLS key shares (`/guardian/v1/validate-custody`), including verifying the validator enclave's CVM session evidence on-chain via `SessionRegistry`
- Signing voluntary exit messages with the guardian's BLS key share (`/guardian/v1/sign-exit`)

**Used by**: [Reef](https://github.com/PufferFinance/reef) (reef-guardian) via the `GuardianClientTrait` HTTP client.

**Deployment**: Must be deployed to GCP with TDX for attestation to work. The CVM Agent handles TDX DCAP attestation and signs payloads with the session key.

### `validator`

The validator binary runs the validator enclave RPC server. It is responsible for:

- Generating attested BLS keys with threshold secret sharing (`/bls/v1/keygen`)
- Listing BLS keystores (`/eth/v1/keystores`)
- General-purpose BLS signing following the [Web3Signer](https://consensys.github.io/web3signer/web3signer-eth2.html) spec (`/api/v1/eth2/sign/:bls_pk_hex`) — compatible with consensus clients

**Used by**: [Coral](https://github.com/PufferFinance/coral) (coral-cli) via the `ValidatorClientTrait` HTTP client, when an `enclave_url` is provided.

**Deployment**: This binary is for external users who want to run their own TDX infrastructure for additional trust guarantees. When no `enclave_url` is configured, coral-cli performs BLS key generation locally using the `generate_bls_keystore_handler` library function — no deployed validator binary is needed.

## Library usage

Both reef and coral also depend on `puffersecuresigner` as a Rust library crate for:

- **HTTP client implementations** (`ClientBuilder`, `GuardianClient`, `ValidatorClient`) for calling the enclave RPCs
- **Types and crypto logic** (`BlsKeygenPayload`, `SignExitRequest`, `ValidateCustodyRequest`, etc.) — these include methods for BLS key parsing, deposit message verification, and key share decryption
- **Local key generation** (`generate_bls_keystore_handler`) — used by coral-cli in the non-enclave path
- **Eth2 types** (`Fork`, `ForkInfo`, `Root`, etc.)

## Development

Uses [atakit](https://github.com/automata-network/atakit) for building, packaging, and deploying TDX workloads. Install it with:

```bash
git clone https://github.com/automata-network/atakit
cd atakit
just install
```

### Local development with sim-agent

The `atakit sim-agent` command simulates the CVM Agent locally without TDX hardware. It starts an embedded Anvil node, registers temporary workloads, and serves mock `/sign-message` and `/rotate-key` endpoints over Unix sockets.

See [docs/local-development.md](docs/local-development.md) for the full setup guide, including deploying Puffer contracts and whitelisting workload IDs.

```bash
# 1. Start anvil
anvil --fork-url https://rpc.hoodi.ethpandaops.io

# 2. Deploy contracts against anvil (see local-development.md)

# 3. Start sim-agent — registers dev workloads and starts mock CVM Agent
atakit sim-agent --rpc-url http://localhost:8545

# 4. Run services via docker compose
docker compose -f container/guardian/docker-compose.yml up
docker compose -f container/validator/docker-compose.yml up
```

Alternatively, set `CVM_AGENT_STUB=true` to bypass the CVM Agent entirely and return empty attestation evidence (useful for unit tests):

```bash
CVM_AGENT_STUB=true cargo run --bin guardian
```

## Deployment

Workloads are deployed to GCP TDX-capable VMs (`c3-standard-4`) using atakit. The project's `atakit.json` defines workloads for each binary with encrypted 10 GB data disks.

### Build and deploy

```bash
# Build workload package (produces a measured artifact — its hash is the workload_id)
atakit workload build guardian

# Publish workload on-chain
atakit workload publish guardian --rpc-url $RPC_URL --owner-private-key $PRIVATE_KEY

# Deploy to GCP
atakit deploy guardian-tdx --platform gcp
```

### Configuration

Each service has a docker-compose file under `container/<service>/` with:

- **`env`** — environment variables (`RUST_LOG`, port). These are measured and included in the workload hash.
- **`network_config.json`** — fork info and network details. Mounted read-only at `/config/network_config.json`. Update for your target network before deploying.
- **`cvm-agent.sock`** — Unix socket for CVM Agent communication, mounted at `/app/cvm-agent.sock`.
- **Named volume** — encrypted data disk at `/data` for key storage.

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

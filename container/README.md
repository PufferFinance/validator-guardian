# atakit workloads

New here? Follow [the end-to-end setup guide](../docs/local-development.md) first.
This page is the workload configuration reference.

Use the current atakit `master` with manifest format 8 and `automata-linux:v0.3.1-debug`.
The `dev/automata-linux-v0.3.2-debug` builder emits format 9, which this base image's
Portal does not accept. A TOML's `format` field does not downgrade builder output.
Always rebuild and inspect archives after changing toolchains.

| Workload | Production config | Host/container port | Encrypted disk |
| --- | --- | --- | --- |
| guardian | `guardian/atakit-workload.toml` | 9001 / 9001 | guardian-data, 10 GB |
| validator | `validator/atakit-workload.toml` | 9002 / 9001 | validator-data, 10 GB |

Production definitions use version `v0.2.1`; older archives are not updated in place.
Each workload has a separate session and persistent `/data` disk. TPM encryption
binds the disk to platform, base-image, and workload measurements. Existing encrypted
disks need an explicit recovery/resealing plan when those measurements change.

## Build

Configure Hoodi, image repositories, publisher/owner keys, and cloud targets in
your atakit operator configuration.

Run from the repository root:

```bash
atakit workload build -d . --file container/guardian/atakit-workload.toml -o build/workloads
atakit workload build -d . --file container/validator/atakit-workload.toml -o build/workloads
```

`-d .` anchors production image/package paths at the repository root. `--file`
selects a service's config; `BINARY_NAME` selects the binary. Buildx's `--platform`
selects the architecture for both compilation and runtime. The Dockerfile uses
the target platform's compiler and libraries without cross-compilation. It starts
the binary at `/`, so its existing relative `./data` paths use the `/data` mount.
`.dockerignore` excludes runtime data and operator keys from the build context.

Atakit checks Docker archive-export capability and reuses a compatible builder.
If none is available, it asks before automatically creating a `docker-container`
builder. It does not change the global builder selection. `BUILDX_BUILDER` can
select one explicitly; noninteractive sessions need a compatible builder already.
Use `[build].container_engine = "docker"` or `--engine docker` to select Docker.

## Publish and deploy

Building does not publish or deploy. Inspect the actual archive first:

```bash
atakit workload info build/workloads/guardian-v0.2.1.atawl
atakit workload publish build/workloads/guardian-v0.2.1.atawl --chain hoodi
atakit cloud deploy build/workloads/guardian-v0.2.1.atawl \
  --target gcp-c3-standard-4 --chain hoodi \
  --image automata-linux:v0.3.1-debug \
  --name guardian-deploy-test --disk-setup guardian-data=create
```

Verify format 8, publisher, base-image whitelist, measured environment, and disk
policy. The named target and signing keys must exist in your operator config.
For Validator, substitute its archive, instance name, and disk
name. `create` only authorizes preparing an empty disk; even a newly created cloud
disk needs this authorization. Never use `overwrite` to bypass a recovery failure
on a disk holding validator keys or slashing history.

Changing measured configuration requires rebuilding and publishing an unused
workload version. Real deployment uses the Hoodi RPC and production registry;
local emulator registrations and signatures exist only on its private fork.

## Local Emulator

The root [atakit-emulator.toml](../atakit-emulator.toml) selects the same
`container/<service>/atakit-workload.toml` files used for production builds.
There are no separate local workload definitions. Emulator uses local directories
for disks and ignores TPM encryption/unlock requirements without changing the TOML.

Use the current atakit binary, Foundry Anvil, Bash, jq, just, and configured Hoodi
chain plus publisher/owner keys:

```bash
export ATAKIT_BIN=/Volumes/ata/cvm/atakit/target/release/atakit
just dev-anvil
```

Keep that terminal open. In another terminal with the same binary setting:

```bash
just emulator-up --target gcp-c3-standard-4
just emulator-status
just run guardian
```

The emulator creates Anvil B on 8546 and registers the configured workloads locally.
It does not start the applications. Reusing the production identity still requires
compatible registered metadata; a conflicting published version is not bypassed.
The main workload must have `image.build`; pulled `.atawl` packages and registry-only
main images are not yet supported. Dependency images may still come from a registry.
Business workload allowlists remain enforced. Use the IDs from emulator status,
and see [workload identity and business allowlists](../docs/local-development.md#workload-identity-and-business-allowlists)
before reusing production IDs or changing the publisher/version.
When switching from the removed local definitions, stop the old applications and
use `just emulator-down` before starting the new configuration. Data is retained.

All production binaries listen on 9001. Native processes share host ports, so run
one at a time or explicitly override ports when starting additional processes:

```bash
VALIDATOR_PORT=9002 just run validator
```

Native storage resolves under `$EMULATOR_ROOTFS/data`, the Portal socket under
`$EMULATOR_ROOTFS/run`, and verification uses `EMULATOR_RPC_URL`. Without those
variables, existing data paths, socket settings, and production RPC remain in effect.
The Portal client is vendored in `crates/atakit-portal-client` from
`atakit_examples`. Socket selection does not require an environment variable:
CVMs and containers use `/run/atakit-portal.sock`; native Emulator uses
`$EMULATOR_ROOTFS/run/atakit-portal.sock`. The old `ATAKIT_PORTAL_SOCKET_PATH`
and `CVM_AGENT_SOCKET_PATH` overrides are no longer read.

### Compose platform selection

Select a temporary container architecture with `--platform` before `up`:

```bash
just compose guardian --platform linux/amd64 up --build
```

Atakit writes the Compose `platform` field for the workload, dependencies, and
Portal bridges. Docker and compatible Podman Compose providers use this field;
the engine must support the selected architecture. Without the option, the
engine chooses its default. Repeat the option on each `up`; `logs` and `down`
reuse the generated file. See the [local development guide](../docs/local-development.md#compose-is-optional)
for the direct atakit command and how this differs from `workload build`.

### Compose limitations with shared configuration

Compose injects `EMULATOR_RPC_URL` pointing to Anvil B. Guardian reads it before
`SESSION_REGISTRY_RPC_URL`, so the production TOML can retain its Hoodi URL.
Linux host networking uses loopback; other hosts use `host.docker.internal` and
require container-engine support for accessing that host service.

Linux port remapping remains unsupported: the host-network adapter rejects the
Validator’s production `9002:9001` mapping.

Run Emulator from the repository root, just like `atakit workload build -f ...`.
It saves that working directory for build contexts, Dockerfiles, and data paths;
the `just` commands already launch it there. Existing runtimes need down/up to
adopt the new path resolution.

TPM declarations no longer block Compose generation; the port-mapping limitation is separate.
The shared-config Compose flow has not yet been validated end to end.

Stop app processes separately. Stop Compose containers before `just emulator-down`,
which deletes generated Compose files. `just emulator-stop` retains checkpoints;
`just emulator-down` removes chain/session state while retaining data, unless
`--purge-data` is explicitly supplied. Changes to TOML/measured data require down/up;
`emulator refresh` updates the upstream fork but does not reload workload inputs.
Keep `.atakit-emulator/` out of version control. The user-owned Anvil state is
stored in `.atakit-emulator/hoodi-anvil.json` and retained by `emulator down`.

## Verification boundary

The Portal adapter validates signing-domain, session-key, fingerprint, and signature
responses. Custody still calls `verifySessionSignature` and checks workload policy.
Proof submitters supply the owner's public identity separately; a session key is
not the owner key. Mock hardware in Emulator does not validate TDX attestation,
TPM disk recovery, or production deployment. Historical migration results are in
[the migration record](../docs/atakit-migration-status.md).

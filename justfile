set positional-arguments
set shell := ["bash", "-eu", "-o", "pipefail", "-c"]

atakit := env_var_or_default("ATAKIT_BIN", "atakit")

default:
    @just --list

# Fork the configured Hoodi chain and persist the user-owned Anvil state.
dev-anvil *args:
    #!/usr/bin/env bash
    set -euo pipefail
    upstream=$("{{atakit}}" emulator status --json | jq -er '.chains.hoodi.rpc_url | select(type == "string" and length > 0)')
    mkdir -p .atakit-emulator
    exec anvil --fork-url "$upstream" --host 127.0.0.1 --port 8545 \
        --hardfork osaka --preserve-historical-states \
        --state .atakit-emulator/hoodi-anvil.json --state-interval 10 "$@"

# Start the shared emulator for all three workloads.
emulator-up *args:
    @"{{atakit}}" emulator up "$@"

# Inspect local sessions and exported connection information.
emulator-status *args:
    @"{{atakit}}" emulator status "$@"

# Stop the emulator while preserving sessions and data.
emulator-stop *args:
    @"{{atakit}}" emulator stop "$@"

# Remove emulator state, retaining application data unless --purge-data is passed.
emulator-down *args:
    @"{{atakit}}" emulator down "$@"

# Run one native binary with its own emulator socket, data, and RPC.
run workload *args:
    #!/usr/bin/env bash
    set -euo pipefail
    workload=$1
    shift
    case "$workload" in guardian|validator|secure-signer) ;; *) echo 'Select guardian, validator, or secure-signer.' >&2; exit 1 ;; esac
    exec "{{atakit}}" emulator exec \
        --workload "$workload" -- cargo run --locked --bin "$workload" "$@"

# Forward Compose commands for one workload, e.g. just compose guardian up -d --build.
compose workload *args:
    #!/usr/bin/env bash
    set -euo pipefail
    workload=$1
    shift
    case "$workload" in guardian|validator|secure-signer) ;; *) echo 'Select guardian, validator, or secure-signer.' >&2; exit 1 ;; esac
    exec "{{atakit}}" emulator workload-compose \
        --workload "$workload" "$@"

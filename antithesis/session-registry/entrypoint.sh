#!/usr/bin/env sh
set -eu

# session-registry: the on-chain verifier the guardian calls via `eth_call`
# (SessionRegistry.verifySessionSignature / getSession) and the contract the
# workload submits provision_node to (GuardianModule).
#
# For setup this is a plain, healthy anvil node. `--host 0.0.0.0` is required so
# the guardian and workload containers can reach it over the compose network.

anvil \
  --host 0.0.0.0 \
  --port 8545 \
  --chain-id 31337 \
  --accounts 10 \
  --silent &
ANVIL_PID=$!

# Wait for anvil to accept RPC before doing anything else.
until cast block-number --rpc-url http://localhost:8545 >/dev/null 2>&1; do
  sleep 0.5
done
echo "session-registry: anvil is up on :8545 (chain-id 31337)"

# -----------------------------------------------------------------------------
# TODO(antithesis-workload): deploy the real contracts here.
#
# Deploy SessionRegistry + GuardianModule from puffer-contracts and rotate in the
# guardian's runtime enclave address (see scratchbook/deployment-topology.md,
# refinements R2 and R7). This is workload-scoped because the guardian's enclave
# address is only known after a runtime `POST /eth/v1/keygen`, so guardian
# registration must be driven by the workload, not baked in at image build.
#
# Until then the node comes up empty; the guardian's verify-session path is not
# exercised (the workload gates verify_session=false, matching production today).
# -----------------------------------------------------------------------------

# Keep the node in the foreground so the container stays up and signals/crashes
# are handled by the compose init process.
wait "$ANVIL_PID"

#!/usr/bin/env sh
set -eu

# Workload / driver entrypoint.
#
# For setup this container is deliberately minimal: it waits until the guardian
# (SUT) and session-registry (dependency) are reachable, signals Antithesis that
# setup is complete, then idles so test-template commands can run against a live
# client. The `antithesis-workload` skill replaces this with the real
# puffersecuresigner-backed Rust driver + SDK assertions.

GUARDIAN_URL="${GUARDIAN_URL:-http://guardian:9001/upcheck}"
RPC_URL="${SESSION_REGISTRY_RPC_URL:-http://session-registry:8545}"

echo "workload: waiting for guardian at ${GUARDIAN_URL}"
until curl -fsS "${GUARDIAN_URL}" >/dev/null 2>&1; do
  sleep 1
done
echo "workload: guardian is healthy"

echo "workload: waiting for session-registry at ${RPC_URL}"
until curl -fsS -X POST -H 'content-type: application/json' \
  --data '{"jsonrpc":"2.0","id":1,"method":"eth_blockNumber","params":[]}' \
  "${RPC_URL}" >/dev/null 2>&1; do
  sleep 1
done
echo "workload: session-registry is healthy"

# Tell Antithesis (and `snouty validate`) the system is up and ready for tests.
/opt/antithesis/setup-complete.sh

echo "workload: setup complete; idling (antithesis-workload will add test drivers)"
# Stay alive so Antithesis can run test-template commands against a live client.
exec tail -f /dev/null

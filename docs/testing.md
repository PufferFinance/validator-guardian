# Local API testing

Use the [local development guide](local-development.md#run-guardian-and-validator-together)
to start Anvil, Emulator, Guardian on port 9001, and Validator on port 9002.
Attestation requests in this workflow use Portal.
This flow uses one Guardian and threshold 1. It does not cover multi-Guardian aggregation.

## Automated checks

```bash
cargo test --locked -p atakit-portal-client
cargo test --locked --test portal_client --test session_registry_abi
```

These tests cover the Portal protocol, signature/identity validation, and registry
ABI. They do not validate hardware attestation.

## Prepare request data

Run from the repository root with both services running:

```bash
guardian_test_dir=$(mktemp -d)
export VALIDATOR_WORKLOAD_ID=$(just emulator-status | jq -er '.workloads.validator.workload_id')
curl -fsS -o /dev/null -w 'Guardian HTTP %{http_code}\n' http://127.0.0.1:9001/upcheck
curl -fsS -o /dev/null -w 'Validator HTTP %{http_code}\n' http://127.0.0.1:9002/upcheck
```

Expect HTTP 200. The temporary directory holds API responses; service keys and
shares remain in their separate emulator data directories.

## Guardian ETH keygen

```bash
curl -fsS -w '\nHTTP %{http_code}\n' \
  -H 'Content-Type: application/json' \
  -d '{"block_number":1,"guardian_module_address":"0x0000000000000000000000000000000000000001","chain_id":560048}' \
  -o "$guardian_test_dir/guardian-key.json" \
  http://127.0.0.1:9001/eth/v1/keygen

jq . "$guardian_test_dir/guardian-key.json"
```

Expect HTTP 201 and nonempty session/signature/owner evidence. The block number
and module address in this API-only example are placeholders; do not submit this
proof to a business contract. Contract tests must use the actual module address,
block and business parameters.

## Validator BLS keygen

```bash
jq -n --slurpfile guardian "$guardian_test_dir/guardian-key.json" '{
  guardianPubkeys: [$guardian[0].pk_hex],
  withdrawalCredentials: "0100000000000000000000001111111111111111111111111111111111111111",
  threshold: 1,
  forkVersion: [16, 0, 9, 16],
  doRemoteAttestation: true
}' > "$guardian_test_dir/validator-request.json"

curl -fsS -w '\nHTTP %{http_code}\n' \
  -H 'Content-Type: application/json' \
  --data-binary @"$guardian_test_dir/validator-request.json" \
  -o "$guardian_test_dir/validator-key.json" \
  http://127.0.0.1:9002/bls/v1/keygen

jq '{bls_pub_key, deposit_data_root, shares: (.bls_enc_priv_key_shares | length), session_id}' \
  "$guardian_test_dir/validator-key.json"
```

Expect HTTP 201, one encrypted share, and a nonempty session ID. The fork version
is Hoodi's `0x10000910`. Withdrawal credentials here are test data, not deposit instructions.

## Guardian custody

`guardianEnclavePublicKey` expects Base64 public-key bytes; keygen's `pk_hex` and
`guardianPubkeys` use hexadecimal. Convert the Guardian key before submitting:

```bash
guardian_test_pk_base64=$(
  jq -r '.pk_hex | ltrimstr("0x")' "$guardian_test_dir/guardian-key.json" \
    | xxd -r -p | base64 | tr -d '\n'
)

jq -n --slurpfile validator "$guardian_test_dir/validator-key.json" \
  --arg workload "$VALIDATOR_WORKLOAD_ID" --arg guardian "$guardian_test_pk_base64" '{
  keygenPayload: $validator[0],
  guardianEnclavePublicKey: $guardian,
  workloadId: $workload,
  verifySession: true,
  validatorIndex: 0
}' > "$guardian_test_dir/custody-request.json"

curl -fsS -w '\nHTTP %{http_code}\n' \
  -H 'Content-Type: application/json' \
  --data-binary @"$guardian_test_dir/custody-request.json" \
  -o "$guardian_test_dir/custody-response.json" \
  http://127.0.0.1:9001/guardian/v1/validate-custody

jq . "$guardian_test_dir/custody-response.json"
```

Expect HTTP 200, an `enclaveSignature`, and a `blsPubKey` matching Validator's
`bls_pub_key`. Guardian verifies the Validator session on Anvil B, validates the
deposit signature, decrypts and checks its share, then stores it and signs custody.

## Rejection checks

```bash
jq '.workloadId = ("0x" + ("00" * 32))' "$guardian_test_dir/custody-request.json" \
  > "$guardian_test_dir/wrong-workload.json"
curl -sS -w '\nHTTP %{http_code}\n' -H 'Content-Type: application/json' \
  --data-binary @"$guardian_test_dir/wrong-workload.json" \
  http://127.0.0.1:9001/guardian/v1/validate-custody

jq '.keygenPayload.session_id = ""' "$guardian_test_dir/custody-request.json" \
  > "$guardian_test_dir/missing-session.json"
curl -sS -w '\nHTTP %{http_code}\n' -H 'Content-Type: application/json' \
  --data-binary @"$guardian_test_dir/missing-session.json" \
  http://127.0.0.1:9001/guardian/v1/validate-custody
```

Both requests must fail. Business-contract tests should separately reject valid
proofs from unlisted workloads; see the [demo verification flow](local-development.md#verify-a-portal-signature-through-your-deployed-contract-optional).
Stop applications with Ctrl-C; `emulator stop` preserves sessions and data.

## Emulator sessions (2026-09-23)

Historical validation with the earlier separate local TOMLs covered all three
ready sessions, health checks, Guardian/Validator keygen, successful custody with
session verification, and rejection of a wrong workload ID. No public-chain
transaction or cloud deployment was performed. Compose configuration was checked,
but container startup was not verified after a Docker Hub metadata timeout.
These are historical results, not a run of the current guide.

For production acceptance, repeat the API flow against real CVMs and the public
registry with real business parameters. Verify rejected signatures/workload IDs,
session expiration, and encrypted-disk recovery. Emulator synthesizes hardware
evidence, so local success does not establish production code measurements or
hardware authenticity.

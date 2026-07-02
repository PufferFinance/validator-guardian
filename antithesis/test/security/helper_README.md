# Test template: `security`

Maps to `/opt/antithesis/test/v1/security/` in the `workload` container.

Commands (compiled Rust binaries from `antithesis/workload/`, placed here by
`antithesis/Dockerfile`):

- `singleton_driver_unauthenticated_request` — property `no-request-authentication`.
  Probes a randomly-chosen privileged guardian endpoint with no credentials and
  fires `Reachable("guardian served a privileged request with no authentication
  present")` when the endpoint serves it (status != 401/403).

`helper_`-prefixed files (like this one) are ignored by Antithesis.

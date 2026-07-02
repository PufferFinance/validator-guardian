# Test templates

This directory is baked into the `workload` image at `/opt/antithesis/test/v1/`.

It is currently empty of real test commands — `antithesis-setup` only wires the
path. The `antithesis-workload` skill adds test templates here: each template is
a subdirectory of executable command files whose names carry a valid prefix
(`parallel_driver_`, `singleton_driver_`, `serial_driver_`, `first_`,
`eventually_`, `finally_`, `anytime_`).

Files/dirs prefixed with `helper_` (like this one) are ignored by Antithesis.

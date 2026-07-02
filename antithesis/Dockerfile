# Antithesis harness images for the validator-guardian SUT.
#
# Build context is the repo root (see antithesis/config/docker-compose.yaml).
# Named stages produce three images:
#   - guardian          : the SUT, instrumented Rust build + Antithesis SDK
#   - workload          : the client/driver (waits, signals setup_complete, idles)
#   - session-registry  : anvil, the on-chain dependency the guardian eth_call's
#
# See antithesis/AGENTS.md for what is intentionally deferred to antithesis-workload.

# ---------------------------------------------------------------------------
# Guardian (SUT) builder — instrumented with Antithesis coverage
# ---------------------------------------------------------------------------
# NOTE: we deliberately do NOT use `cargo chef` here. `cargo chef cook` compiles
# the crate with stubbed-out source, which drops the `use antithesis_instrumentation
# as _;` reference in lib.rs. Without it, Cargo drops the instrumentation crate and
# the sancov runtime (__sanitizer_cov_trace_pc_guard) is never linked, so the build
# fails to link. A direct build keeps the real source (and that reference) in the graph.
FROM rust:1.91-bookworm AS guardian-builder
WORKDIR /app
# Supplies the sancov coverage rustflags + pins the target. Paired with the
# `antithesis_instr` feature (which pulls in antithesis-instrumentation). Both required.
COPY antithesis/instrumented-cargo-config.toml /app/.cargo/config.toml
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/app/target \
    cargo build --release --bin guardian --features antithesis_instr \
    && cp /app/target/x86_64-unknown-linux-gnu/release/guardian /guardian \
    # Fail the build early unless coverage actually linked: the libvoidstar loader
    # AND the sancov runtime symbol must both be present (the latter proves the
    # sancov codegen ran and its runtime is linked, not just that the SDK is present).
    && nm /guardian | grep -q ' antithesis_load_libvoidstar' \
    && nm /guardian | grep -q ' __sanitizer_cov_trace_pc_guard' \
    && echo "instrumentation OK: libvoidstar loader + sancov runtime linked"

# ---------------------------------------------------------------------------
# Guardian (SUT) runtime
# ---------------------------------------------------------------------------
FROM debian:bookworm-slim AS guardian
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates libssl3 curl \
    && rm -rf /var/lib/apt/lists/*

# NO_COLOR: Antithesis stores raw bytes and does not render ANSI escape codes.
ENV NO_COLOR=1 \
    RUST_LOG=info \
    GUARDIAN_PORT=9001 \
    GENESIS_FORK_VERSION=00000000

# NOTE: intentionally NO WORKDIR. The guardian writes keys to ./data/keys/* ,
# which must resolve to /data (the mounted volume) with CWD = / , matching the
# production deployment (see scratchbook/sut-analysis.md §3).
COPY --from=guardian-builder /guardian /usr/local/bin/guardian

# Expose DWARF/build-id symbols to Antithesis. The release binary is unstripped
# and built with -Cdebuginfo=1 -Wl,--build-id, so a symlink is sufficient.
RUN mkdir -p /data/keys/bls_keys /data/keys/eth_keys /data/slashing /symbols \
    && ln -s /usr/local/bin/guardian /symbols/guardian

EXPOSE 9001
# Exec-form so the guardian is a direct child of the compose init process and
# receives SIGTERM/SIGINT for clean-termination testing (topology R6).
ENTRYPOINT ["/usr/local/bin/guardian"]

# ---------------------------------------------------------------------------
# Workload (client / driver) builder — compiles the Rust test commands
# ---------------------------------------------------------------------------
# The workload is the client, not the SUT, so it is NOT instrumented (no sancov
# rustflags, no antithesis-instrumentation crate). It only needs the antithesis
# SDK for assertions + deterministic randomness. Built as a standalone crate so
# no Antithesis-only code lives in the puffersecuresigner package.
FROM rust:1.91-bookworm AS workload-builder
WORKDIR /app
COPY antithesis/workload/ ./
# registry/git caches are shared with the guardian build (same crates); only the
# target dir is isolated (id=workload-target) because rustflags differ.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/app/target,id=workload-target \
    cargo build --release --bin unauthenticated_request \
    && cp /app/target/release/unauthenticated_request /unauthenticated_request

# ---------------------------------------------------------------------------
# Workload (client / driver) runtime
# ---------------------------------------------------------------------------
FROM debian:bookworm-slim AS workload
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*
ENV NO_COLOR=1
COPY antithesis/setup-complete.sh /opt/antithesis/setup-complete.sh
COPY antithesis/workload/entrypoint.sh /opt/antithesis/entrypoint.sh
# Test templates are discovered by Antithesis / `snouty validate` under this path.
# The helper docs come from antithesis/test/; the executable test commands are the
# compiled Rust binaries placed with their required prefixes.
COPY antithesis/test/ /opt/antithesis/test/v1/
COPY --from=workload-builder /unauthenticated_request \
    /opt/antithesis/test/v1/security/singleton_driver_unauthenticated_request
RUN chmod +x /opt/antithesis/setup-complete.sh /opt/antithesis/entrypoint.sh \
    /opt/antithesis/test/v1/security/singleton_driver_unauthenticated_request
ENTRYPOINT ["/opt/antithesis/entrypoint.sh"]

# ---------------------------------------------------------------------------
# session-registry (anvil) — the guardian's on-chain eth_call dependency
# ---------------------------------------------------------------------------
FROM ghcr.io/foundry-rs/foundry:stable AS session-registry
ENV NO_COLOR=1
USER root
COPY antithesis/session-registry/entrypoint.sh /usr/local/bin/entrypoint.sh
RUN chmod +x /usr/local/bin/entrypoint.sh
EXPOSE 8545
ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]

# Stage 1: Chef - dependency caching
FROM rust:1.91-bookworm AS chef
RUN cargo install cargo-chef
WORKDIR /app

# Stage 2: Planner
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# Stage 3: Builder
FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    cargo chef cook --release --recipe-path recipe.json
COPY . .
ARG BINARY_NAME=secure-signer
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    cargo build --release --bin ${BINARY_NAME}

# Stage 4: Runtime
FROM debian:bookworm-slim AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates libssl3 \
    && rm -rf /var/lib/apt/lists/*

ARG BINARY_NAME=secure-signer
ENV BINARY_NAME=${BINARY_NAME}
COPY --from=builder /app/target/release/${BINARY_NAME} /usr/local/bin/${BINARY_NAME}

# Create data directories
RUN mkdir -p /data/keys/bls_keys /data/keys/eth_keys /data/slashing

EXPOSE 9001

ENTRYPOINT /usr/local/bin/${BINARY_NAME}

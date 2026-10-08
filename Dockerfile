# Base images are pinned by digest; Dependabot (docker) proposes refreshed digests weekly.
FROM rust:1.92-bookworm@sha256:e90e846de4124376164ddfbaab4b0774c7bdeef5e738866295e5a90a34a307a2 AS build
RUN apt-get update && apt-get install -y --no-install-recommends protobuf-compiler pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY sdk/rust ./sdk/rust
COPY proto ./proto
RUN cargo build --release --locked --bins
FROM debian:bookworm-slim@sha256:7c7b2c966bc9ee8cedfeef67e0e279108992c77681fa595db4a9d65c06ccc587
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates libssl3 && rm -rf /var/lib/apt/lists/* && useradd --uid 10001 --create-home geoledger && mkdir /data && chown 10001:10001 /data
COPY --from=build /src/target/release/geoledger-server /src/target/release/gl /usr/local/bin/
USER 10001:10001
VOLUME /data
ENV GL_DATA_DIR=/data GL_HTTP_LISTEN=0.0.0.0:7881 GL_GRPC_LISTEN=0.0.0.0:7882
EXPOSE 7881 7882
# `probe` reads the same GL_* settings (GL_HEALTH_LISTEN when set) and checks /ready.
HEALTHCHECK --interval=15s --timeout=6s --start-period=30s --retries=3 CMD ["geoledger-server", "probe"]
ENTRYPOINT ["geoledger-server"]

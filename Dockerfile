# Base images are pinned by digest; Dependabot proposes updates.
FROM rust:1.98-alpine3.24@sha256:7cc1c22d77d9432f7fe012a70e6d3e555af54c2a6832700ed7d553f1769ae89f AS build
RUN apk add --no-cache build-base pkgconf openssl-dev protobuf-dev protobuf libspatialite
# MUSL defaults to static CRT; dynamic CRT is required for SQLite extension dlopen.
ENV RUSTFLAGS="-C target-feature=-crt-static" GL_SPATIALITE_EXTENSION=/usr/lib/mod_spatialite.so.8
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY sdk/rust ./sdk/rust
COPY proto ./proto
RUN cargo build --release --locked --bins
# The root lock and resolved graph describe the actual build. Cached upstream
# crates' own development lockfiles are not GeoLedger dependency resolutions.
RUN mkdir -p /src/build-evidence && \
    cargo build --release --locked --offline --bins --message-format=json > /src/build-evidence/cargo-artifacts.jsonl && \
    rustc -vV > /src/build-evidence/rustc.txt && \
    rm -rf /usr/local/cargo/registry /usr/local/cargo/git
FROM alpine:3.24@sha256:294b683cb724975bec92580e1e685676bd4b50bda910ddb8c51d4cabeaec77e6 AS runtime
RUN apk add --no-cache ca-certificates libssl3 libcrypto3 libgcc libstdc++ libspatialite && \
    addgroup -g 10001 geoledger && adduser -D -u 10001 -G geoledger geoledger && \
    mkdir /data && chown 10001:10001 /data
COPY --from=build /src/target/release/geoledger-server /src/target/release/gl /usr/local/bin/
COPY Cargo.lock /usr/share/geoledger/Cargo.lock
USER 10001:10001
VOLUME /data
ENV GL_DATA_DIR=/data GL_HTTP_LISTEN=0.0.0.0:7881 GL_GRPC_LISTEN=0.0.0.0:7882 GL_SPATIALITE_EXTENSION=/usr/lib/mod_spatialite.so.8
EXPOSE 7881 7882
HEALTHCHECK --interval=15s --timeout=6s --start-period=30s --retries=3 CMD ["geoledger-server", "probe"]
ENTRYPOINT ["geoledger-server"]

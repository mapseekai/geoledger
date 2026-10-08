FROM rust:1.92-bookworm AS build
RUN apt-get update && apt-get install -y --no-install-recommends protobuf-compiler pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY sdk/rust ./sdk/rust
COPY proto ./proto
RUN cargo build --release --locked --bins
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates libssl3 && rm -rf /var/lib/apt/lists/* && useradd --uid 10001 --create-home geoledger && mkdir /data && chown 10001:10001 /data
COPY --from=build /src/target/release/geoledger-server /src/target/release/gl /usr/local/bin/
USER 10001:10001
VOLUME /data
ENV GL_DATA_DIR=/data
EXPOSE 7881 7882
ENTRYPOINT ["geoledger-server"]
CMD ["--http", "0.0.0.0:7881", "--grpc", "0.0.0.0:7882"]

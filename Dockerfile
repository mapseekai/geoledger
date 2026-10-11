# syntax=docker/dockerfile:1
# Both services share Debian's runtime ABI; all external bases are digest-pinned.
FROM rust:1.98-trixie@sha256:a8a5f0a1e5fe7dfe1d352591e4a1c7dd2c08fd70475cae872cf3458ba0df0546 AS server-build
RUN apt-get update && apt-get install -y --no-install-recommends protobuf-compiler libsqlite3-mod-spatialite && rm -rf /var/lib/apt/lists/*
ENV CARGO_BUILD_JOBS=2
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY sdk/rust ./sdk/rust
COPY proto ./proto
RUN cargo build --release --locked --bins
RUN mkdir -p /src/build-evidence && \
    cargo build --release --locked --offline --bins --message-format=json > /src/build-evidence/cargo-artifacts.jsonl && \
    rustc -vV > /src/build-evidence/rustc.txt && \
    rm -rf /usr/local/cargo/registry /usr/local/cargo/git
# Debug adds only BusyBox for npm lifecycle scripts; this stage is never shipped.
FROM gcr.io/distroless/nodejs22-debian13:debug@sha256:ee2f5ba5b847b26d899406503c8c7e64b71b55adb0bbaa3538f15110bf391493 AS toolchain
SHELL ["/busybox/sh", "-ec"]
ENV PATH=/nodejs/bin:/opt/npm/bin:/busybox:/usr/bin npm_config_script_shell=/busybox/sh
# Pinned npm archive: checksum verified by BuildKit before any archive is executed.
ADD --checksum=sha256:6666b48816b39b86c3febac7b51a4ee4de6c5ca589c382ad8004b6b113f86677 https://registry.npmjs.org/npm/-/npm-12.2.0.tgz /tmp/npm.tgz
ADD --checksum=sha256:ef8448ec78f20b692f04fa6d01f39b5ab34c66404bea3429f5a39c6c9e0be8b4 https://registry.npmjs.org/brace-expansion/-/brace-expansion-5.0.12.tgz /tmp/brace-expansion.tgz
ADD --checksum=sha256:d75e1e6a11587954da5e2f0e2b5c4b397a16d28cc2f7bdf64e9027fc2fe593ee https://registry.npmjs.org/http-cache-semantics/-/http-cache-semantics-4.3.0.tgz /tmp/http-cache-semantics.tgz
ADD --checksum=sha256:e18191aac9c0ff43dac7fe9b10b7041a22d07addb7b66a6e8ac14a52a5b69b74 https://registry.npmjs.org/undici/-/undici-6.28.1.tgz /tmp/undici.tgz
COPY .security/node-build.lock.json scripts/verify-node-build.cjs /opt/build-policy/
# Refresh bundled transitive packages before executing npm for the first time.
RUN mkdir -p /opt/npm /usr/bin /bin && ln -sf /busybox/env /usr/bin/env && ln -sf /busybox/sh /bin/sh && \
    tar -xzf /tmp/npm.tgz -C /opt/npm --strip-components=1 && \
    for package in brace-expansion http-cache-semantics undici; do \
      rm -rf "/opt/npm/node_modules/$package"; mkdir -p "/opt/npm/node_modules/$package"; \
      tar -xzf "/tmp/$package.tgz" -C "/opt/npm/node_modules/$package" --strip-components=1; \
    done && rm /tmp/*.tgz && ln -sf npm-cli.js /opt/npm/bin/npm && node /opt/build-policy/verify-node-build.cjs && npm --version
FROM toolchain AS console-build
# Keep the native TypeScript source/version, but rebuild its embedded Go runtime.
# Compiler tools and their full inventories remain in this separately audited stage.
COPY .security/typescript-build.lock.json .security/typescript-go.mod .security/typescript-go.sum scripts/rebuild-typescript.cjs /opt/build-policy/
RUN node /opt/build-policy/rebuild-typescript.cjs build
WORKDIR /app
COPY sdk/ts ./sdk/ts
COPY web ./web
ENV NEXT_TELEMETRY_DISABLED=1
# Build-stage inventories remain available to the gate; only /output enters runtime.
# proxy_ca is optional for ordinary builds and supplies managed-cloud CA trust.
RUN --mount=type=secret,id=proxy_ca \
    if [ -f /run/secrets/proxy_ca ]; then export NODE_EXTRA_CA_CERTS=/run/secrets/proxy_ca; fi; \
    npm ci --ignore-scripts --prefix sdk/ts && \
    node /opt/build-policy/rebuild-typescript.cjs install /app/sdk/ts && \
    npm --prefix sdk/ts run build && \
    npm ci --ignore-scripts --prefix web && \
    node /opt/build-policy/rebuild-typescript.cjs install /app/web && \
    npm --prefix web run build && \
    mkdir /output && \
    cp -a web/.next/standalone/. /output/ && \
    mv web/.next/static /output/web/.next/static && \
    cp -a web/public /output/web/public

# One complete build inventory is scanned before the runtime is assembled.
FROM server-build AS build
COPY --from=console-build /opt /opt
COPY --from=console-build /app /app
COPY --from=console-build /output /output
FROM gcr.io/distroless/nodejs22-debian13:nonroot@sha256:ec2313763dd43931543bd03830466e0c409ce73a487e8d46f10db72d3b816c1c AS node-runtime
FROM debian:trixie-slim@sha256:a29215f6a35e51e22adffa17f89e9d2ef06214e64a2bad10d765c46aea49f11f AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates libsqlite3-mod-spatialite libssl3t64 libstdc++6 libatomic1 tini && \
    rm -rf /var/lib/apt/lists/* && \
    groupadd --gid 10001 geoledger && useradd --uid 10001 --gid 10001 --no-create-home geoledger && \
    mkdir /data && chown 10001:10001 /data
COPY --from=node-runtime /nodejs /nodejs
COPY --from=build /src/target/release/geoledger-server /src/target/release/gl /usr/local/bin/
COPY --from=build --chown=10001:10001 /output /app
COPY Cargo.lock /usr/share/geoledger/Cargo.lock
COPY deploy/container.cjs /app/container.cjs
WORKDIR /app
USER 10001:10001
VOLUME /data
ENV GL_DATA_DIR=/data GL_HTTP_LISTEN=0.0.0.0:7881 GL_GRPC_LISTEN=0.0.0.0:7882 \
    GL_WEB_ENDPOINT=http://127.0.0.1:7882 NODE_ENV=production NEXT_TELEMETRY_DISABLED=1 HOSTNAME=0.0.0.0 PORT=3000
EXPOSE 3000 7881 7882
HEALTHCHECK --interval=15s --timeout=8s --start-period=30s --retries=3 CMD ["/nodejs/bin/node", "/app/container.cjs", "probe"]
ENTRYPOINT ["/usr/bin/tini", "--", "/nodejs/bin/node", "/app/container.cjs"]

# syntax=docker/dockerfile:1
# Both services share musl; retain APK inventories for native dependency scans.
FROM rust:1.98-alpine3.24@sha256:7cc1c22d77d9432f7fe012a70e6d3e555af54c2a6832700ed7d553f1769ae89f AS server-build
RUN apk upgrade --no-cache && apk add --no-cache build-base cmake pkgconf openssl-dev protobuf-dev protobuf libspatialite 'brotli-libs>=1.2.1-r0'
# Dynamic CRT is required for SpatiaLite extension loading.
ENV CARGO_BUILD_JOBS=2 RUSTFLAGS="-C target-feature=-crt-static" GL_SPATIALITE_EXTENSION=/usr/lib/mod_spatialite.so.8
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
# Node 22.23.4 backports CVE-2026-111497; its bundled Brotli still reports 1.1.0.
FROM node:26.12.1-alpine3.24@sha256:40cd011b55415bc894e45b02fdd5a5dfde056921b7d23a64c74f80497af48797 AS node-runtime
RUN node -e 'require("node:assert/strict").equal(process.versions.node, "22.23.4")'
FROM alpine:3.24@sha256:be2656061b536c1883c4034790e21d0a09c5aaaf51968b89368a3c0371ea2c9a AS toolchain
RUN apk upgrade --no-cache && apk add --no-cache ca-certificates libstdc++
COPY --from=node-runtime /usr/local/bin/node /nodejs/bin/node
SHELL ["/bin/sh", "-ec"]
ENV PATH=/nodejs/bin:/opt/npm/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin npm_config_script_shell=/bin/sh
# Pinned npm archive: checksum verified by BuildKit before any archive is executed.
ADD --checksum=sha256:6666b48816b39b86c3febac7b51a4ee4de6c5ca589c382ad8004b6b113f86677 https://registry.npmjs.org/npm/-/npm-12.2.0.tgz /tmp/npm.tgz
ADD --checksum=sha256:ef8448ec78f20b692f04fa6d01f39b5ab34c66404bea3429f5a39c6c9e0be8b4 https://registry.npmjs.org/brace-expansion/-/brace-expansion-5.0.12.tgz /tmp/brace-expansion.tgz
ADD --checksum=sha256:d75e1e6a11587954da5e2f0e2b5c4b397a16d28cc2f7bdf64e9027fc2fe593ee https://registry.npmjs.org/http-cache-semantics/-/http-cache-semantics-4.3.0.tgz /tmp/http-cache-semantics.tgz
ADD --checksum=sha256:29c4b8e0b2d86cb8795571c1959012ce057a14ecfd34858335f47c31e770a04f https://registry.npmjs.org/undici/-/undici-6.29.1.tgz /tmp/undici.tgz
ADD --checksum=sha256:f9e1d0dd369c002f43770c6ee35942c8078e76d080a1e18e89113eef865f26ff https://registry.npmjs.org/ip-address/-/ip-address-10.7.4.tgz /tmp/ip-address.tgz
ADD --checksum=sha256:48c903f98c5591d8d26f16f389d5cb3435459474c645209a79f6241bf4b657b6 https://registry.npmjs.org/postcss-selector-parser/-/postcss-selector-parser-7.1.6.tgz /tmp/postcss-selector-parser.tgz
COPY .security/node-build.lock.json scripts/verify-node-build.cjs /opt/build-policy/
# Refresh bundled transitive packages before executing npm for the first time.
RUN mkdir -p /opt/npm && \
    tar -xzf /tmp/npm.tgz -C /opt/npm --strip-components=1 && \
    for package in brace-expansion http-cache-semantics undici ip-address postcss-selector-parser; do \
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
# Lint runs in CI; remove unused native linters from this build without changing locks.
# proxy_ca is optional for ordinary builds and supplies managed-cloud CA trust.
RUN --mount=type=secret,id=proxy_ca \
    if [ -f /run/secrets/proxy_ca ]; then export NODE_EXTRA_CA_CERTS=/run/secrets/proxy_ca; fi; \
    npm ci --ignore-scripts --prefix sdk/ts && \
    npm uninstall --ignore-scripts --no-save --prefix sdk/ts oxlint oxlint-tsgolint && \
    node /opt/build-policy/rebuild-typescript.cjs install /app/sdk/ts && \
    npm --prefix sdk/ts run build && \
    npm ci --ignore-scripts --prefix web && \
    npm uninstall --ignore-scripts --no-save --prefix web oxlint oxlint-tsgolint && \
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
# A standalone lock avoids scanner workspace-resolution gaps in build images.
COPY Cargo.lock /usr/share/geoledger/Cargo.lock
FROM alpine:3.24@sha256:be2656061b536c1883c4034790e21d0a09c5aaaf51968b89368a3c0371ea2c9a AS runtime
RUN apk upgrade --no-cache && apk add --no-cache ca-certificates libssl3 libcrypto3 libgcc libstdc++ libspatialite tini 'brotli-libs>=1.2.1-r0' && \
    addgroup -g 10001 geoledger && adduser -D -u 10001 -G geoledger geoledger && \
    mkdir /data && chown 10001:10001 /data
COPY --from=node-runtime /usr/local/bin/node /nodejs/bin/node
COPY --from=build /src/target/release/geoledger-server /src/target/release/gl /usr/local/bin/
COPY --from=build --chown=10001:10001 /output /app
COPY Cargo.lock /usr/share/geoledger/Cargo.lock
COPY deploy/container.cjs /app/container.cjs
WORKDIR /app
USER 10001:10001
VOLUME /data
ENV GL_DATA_DIR=/data GL_HTTP_LISTEN=0.0.0.0:7881 GL_GRPC_LISTEN=0.0.0.0:7882 \
    GL_SPATIALITE_EXTENSION=/usr/lib/mod_spatialite.so.8 GL_WEB_ENDPOINT=http://127.0.0.1:7881 \
    NODE_ENV=production NEXT_TELEMETRY_DISABLED=1 HOSTNAME=0.0.0.0 PORT=3000
EXPOSE 3000 7881 7882
HEALTHCHECK --interval=15s --timeout=8s --start-period=30s --retries=3 CMD ["/nodejs/bin/node", "/app/container.cjs", "probe"]
ENTRYPOINT ["/sbin/tini", "--", "/nodejs/bin/node", "/app/container.cjs"]

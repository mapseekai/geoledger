# syntax=docker/dockerfile:1
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
FROM toolchain AS build
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
# Runtime contains Node and its libraries only, not npm, shells or build tools.
FROM gcr.io/distroless/nodejs22-debian13:nonroot@sha256:ec2313763dd43931543bd03830466e0c409ce73a487e8d46f10db72d3b816c1c AS runtime
WORKDIR /app
ENV NODE_ENV=production NEXT_TELEMETRY_DISABLED=1 HOSTNAME=0.0.0.0 PORT=3000
COPY --from=build --chown=65532:65532 /output ./
USER 65532:65532
EXPOSE 3000
CMD ["web/server.js"]

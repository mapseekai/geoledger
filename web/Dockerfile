# syntax=docker/dockerfile:1
FROM node:22-bookworm-slim AS build
WORKDIR /app
COPY sdk/ts ./sdk/ts
COPY web ./web
ENV NEXT_TELEMETRY_DISABLED=1
# A single build layer avoids retaining multiple copies of dependency trees.
# proxy_ca is optional for ordinary builds and supplies managed-cloud CA trust.
RUN --mount=type=secret,id=proxy_ca \
    if [ -f /run/secrets/proxy_ca ]; then export NODE_EXTRA_CA_CERTS=/run/secrets/proxy_ca; fi; \
    npm ci --prefix sdk/ts && \
    npm --prefix sdk/ts run build && \
    npm ci --prefix web && \
    npm --prefix web run build && \
    mkdir /output && \
    cp -a web/.next/standalone/. /output/ && \
    mv web/.next/static /output/web/.next/static && \
    cp -a web/public /output/web/public && \
    rm -rf sdk/ts/node_modules web/node_modules web/.next
FROM node:22-bookworm-slim AS runtime
WORKDIR /app
ENV NODE_ENV=production NEXT_TELEMETRY_DISABLED=1 HOSTNAME=0.0.0.0 PORT=3000
COPY --from=build --chown=node:node /output ./
USER node
EXPOSE 3000
CMD ["node", "web/server.js"]

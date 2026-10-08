# Development contract

The repository root is the Cargo workspace. This is a service-only product:
geoledger-server owns storage, gl is a remote gRPC client, and Go/Rust/Node.js/Python
SDKs share proto/geoledger/v1/geoledger.proto. Browser management lives in the separate
web/ Next.js app: browser HTTP -> Node.js BFF -> TS business SDK -> gRPC.
Never import the Node SDK into client components. Keep credentials in encrypted
HttpOnly sessions; preserve exact feature JSON and immutable publication retries.

Core stays independent of databases and transports. Every transport calls engine
Application. Application depends on StorageBackend/RepositoryTransaction semantic
operations; backend SQL belongs in engine session adapters, never in business handlers.
New backends must pass the same conformance suite. Preserve atomic publication,
historical visibility, optimistic draft versions, authorization, original-request
idempotency, audit writes, and rollback on dropped or failed transactions.

SQLite is the default server backend with WAL and durable transactions. PostGIS is
selected explicitly. Never fall back to SQLite when an explicit PostGIS configuration
fails. Clients never open a database or local repository.

Run ./scripts/check.sh with Python 3 available. Web changes also require the
checks in web/README.md (Node.js 22), including production build and meaningful
browser verification. SDK regeneration uses
protoc, the pinned Go plugins, grpcio-tools and ts-proto; see docs/development.md.
Opt-in PostGIS tests use GL_TEST_DATABASE_URL and require database geoledger_test.
Keep fixtures in disposable test environments. Capacity tests are explicit opt-in.

Target the current storage and protocol only: no legacy readers or command
aliases. The server never upgrades storage at startup. Structural storage changes
require a new format version plus a forward-only migration in
crates/engine/src/migrations for both backends, applied only by the operator
command `geoledger-server migrate` after a backup and covered by
crates/engine/tests/upgrade.rs. Document measured validation separately from deployment claims.
Do not commit credentials, database files, generated build caches or test logs.

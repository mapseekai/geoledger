# Development contract

The repository root is the Cargo workspace. This is a service-only product:
geoledger-server owns storage, gl is a remote gRPC client, and Go/Rust/Node.js/Python
SDKs share proto/geoledger/v1/geoledger.proto. Browser management lives in the separate
web/ Next.js app: browser HTTP -> Node.js BFF -> private HTTP session transport -> engine.
Never import the Node SDK into client components. Keep only opaque database-backed Session IDs in encrypted
HttpOnly cookies; public SDKs accept API Keys only. Preserve exact feature JSON and immutable publication retries.

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

Scope platform-specific imports, bindings and checks with matching cfg attributes.
Keep Unix credential-file permission and device/inode checks intact; fix Windows
warnings without weakening lints. Run native platform checks where available and
distinguish Linux, cross-compilation and native Windows evidence.

The two console CI browser suites use scripts/browser_test_support.py and keep
Chromium sandboxing enabled. Ubuntu 24.04 CI uses the installed Chrome channel
with its existing AppArmor support. Do not disable browser sandboxing or change
system security restrictions to make tests pass; report an environment blocker.

For dependency security fixes, prefer compatible targeted lockfile updates and
run scripts/audit.sh plus the affected TLS/SDK regressions. Account for Rust,
npm, Go and Python audits even when an earlier scanner stops the script. Record
commands, toolchains, advisory sources and pass/fail/skip/block status explicitly;
an alternate database snapshot is distinct from the live audit endpoint.
Keep validation reports separate from product claims. Keep README.md and
README.en.md aligned, with capabilities and examples grounded in current source.

Target the current storage and protocol only: no legacy readers, command aliases,
or upgrades from old storage formats. Initialize fresh databases directly at the
current format. Structural storage changes require a new format version.
Keep current-format backup, restore, import/export and transaction rollback.
Document measured validation separately from deployment claims.
Do not commit credentials, database files, generated build caches or test logs.

# Dataset collaboration RPC service extraction

User approved the existing MapSeek RPC layering. Replace the collaboration HTTP listener in spatial-engine with:
Browser HTTP → API Gateway → existing Kitex featureserv service → Volo/Thrift geoledger-runtime → embedded geoledger-center crate.

## DDD sketch
Goal/context: Dataset version-control application, reusing GeoLedger drafts, revisions, conflict resolutions and publication receipts.
Invariants: permission revocation locks, dataset/physical collection binding, feature writes, HEAD/history/receipt, Dataset version and tile marker stay in one database transaction. Go validates the request envelope and orchestrates the runtime call; Rust Application remains the owner of these invariants. No duplicated version-control domain model.
Ports/adapters: application request/result and runtime call; Kitex public RPC boundary, Volo native RPC adapter. JSON command/result remain opaque strings to retain exact numeric values.
Idempotency: existing draft chunk and publish request IDs remain unchanged; transport failures never trigger automatic publication retry.
Trust: gateway→Go existing full-request proof; featureserv→Rust length-prefixed SHA256 request binding, signed actor/tenant/project/API-key and Execute method. Each hop has its own caller key and audience. Database checks actual current resource access on every command.
Storage: existing current-format center schema and OPFS cache; no format change or migration.

## Implementation and verification
1. Add idl/dataversion/dataversion.thrift: ExecuteRequest(dataset_uid, collection, command_json), ExecuteResponse(status, body_json), DataVersionService.Execute. Generate Go code with project Kitex command.
2. Extract DatasetHost and its tests into services/featureserv/geoledger-runtime. Use existing Volo/Thrift dependencies, bounded frame/concurrency and 55s DB deadline; explicit prepare command, fail-closed startup.
3. Extend featureserv with a collaboration application, RPC adapters/startup and tests. Forward trusted scope with a new signed proof. Runtime request binding is lowercase SHA256 of each UTF-8 field prefixed by its unsigned 64-bit big-endian byte length, in IDL field order. SignContext payload is that hex string.
4. Replace gateway HTTP collaboration client with configured Kitex client; preserve HTTP API, byte/status guards, error semantics and frontend behavior. Remove obsolete spatial-engine collaboration entrypoint/dependency/env.
5. Register workspace/codegen/local startup and document prerequisites and complete call chain.
6. Run focused Go/Rust checks plus actual gateway/featureserv→Rust Thrift and disposable PostGIS host tests. Preserve prior MinIO and raster fixes. Report measured scope; do not publish crates.io yet.

User amendment: reuse the existing featureserv Go service. Gateway calls FeatureServ.ExecuteCollaboration; featureserv calls DataVersionService.Execute in its private Rust runtime. Runtime caller is featureserv. No additional Go service, Go workspace module or port is introduced.

Implementation status: gateway now uses its existing featureserv client with ExecuteCollaboration and a per-call 65s timeout. Featureserv owns collaboration input validation and calls its embedded-runtime adapter; existing feature request/response transport limits are preserved. Rust execution uses separate bounded frame admission and blocking-job permits. Scope/command signatures, HTTP response forwarding, real signed Kitex→Volo PostGIS workflow and transaction-local host rollback tests passed. Runtime unit tests cover actual codec deadlines, admission release and payload boundaries. Runtime build, six unit/transport tests, explicit PostGIS host fixture, rustfmt and all-target Clippy with warnings denied all passed. The disposable runtime, database container and signing keys were cleaned up.

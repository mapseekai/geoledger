# Progress — 2026-10-07 Dataset collaboration

Approved spec: 2026-10-07-mapseek-dataset-collaboration.md. Execution explicitly authorized by user.

- Baseline GeoLedger: 12eb292, clean except approved design document.
- Baseline MapSeek: preserve existing changes to Muse LayerListItem/tests, ResourceSidebar, maptile pmtiles_generation/tests.
- Both checkouts now use codex/dataset-collaboration; working in place preserves the user's existing workspace and local dependencies.
- Baseline checks from analysis: docs/browser protocol/fmt passed; Cargo network and missing offline dependencies prevented Rust suite; no GL_TEST_DATABASE_URL configured.
- User authorized changes to GeoLedger as needed and local source dependency during implementation, with registry release after completion.
- Shared API is recorded in the implementation plan. Parallel files have separate owners; host integration remains with controller.

## Implementation and verification

- Embedded collaboration library, OPFS cache/session and editor UI are implemented. Native per-field choices preserve unrelated numeric/geometry values; deleted-row restoration preserves original generated IDs and EWKB.
- Gateway signed forwarding and permission tests plus scoped Go tests: 94 passed. Dedicated featureserv PostGIS test passed for old single/batch mutation rejection of managed tables.
- Actual spatial-engine host tests passed for signing, authorization, version/marker atomicity and rollback. Full spatial-engine cargo check and all-target Clippy passed, including DuckDB/GDAL and Temporal dependencies. The original Go-to-Rust proof fixture also passed.
- Full default spatial-engine tests: 110 passed, 3 failed, 42 ignored. Both unchanged raster failures passed in isolated serial reruns. The unchanged MinIO `progress_failure_aborts_remote_multipart_session` test also timed out in isolation; the complete MapSeek suite is not green.
- Follow-up investigation resolved those three failures: process-wide GDAL `EMPTY_DIR` hid external `.msk` files; `TRUE` retains sidecar discovery and explicit production-config regressions now pass. MinIO fixture timing placed the progress error at 8.143 seconds and abort request at 8.147 seconds; debug signing of 32 MiB consumed the test budget. Two legal 5 MiB parts retain concurrent transfer/abort coverage with the unchanged 10-second timeout. Full default parallel spatial-engine tests now pass: 113 passed, 0 failed, 42 opt-in tests ignored.
- GeoLedger `scripts/check.sh` passed again on final source with Node 22 (documentation, browser protocol, formatting, locked workspace Clippy/tests). Native Center PostGIS 16/16 passed. New collaboration database tests 4/4 passed plus one protocol test before review fixes; subsequent focused PostGIS regressions passed for every review fix and the 1201-change allocation fixture. App PostGIS 26/26, Thrift PostGIS 1/1 and PostGIS library 2/2 passed in the dedicated test database.
- Final combined frontend tests: 84 passed across cache, protocol, editor, dialogs, permission and tile modules with one worker; cloud TypeScript check passed. Parallel execution under simultaneous Rust builds produced two 5-second test timeouts; serial execution passed. Active editor transport outages preserve local work; fresh offline entry still needs catalog/initial permission access.
- Browser fixture uses real OPFS + HTTP: initial 1200 features 245213 B; warm HEAD 190 B and no feature download; two-change delta 530 B. These are controlled fixture measurements, not production scalability claims.
- Real Loom browser integration passed stable 35-character project IDs, restored drafts after refresh, HEAD-only reopening, empty-comment property-only publication and empty-layer schema restoration. Browser TaskSpace 50 is shared, p1 cache/p2 editor.
- Real signed Rust/PostGIS browser checks passed independent-field merge for two users, exact numeric values, native field choices, lost-response replay with one insertion, and restore/re-edit/publish with the original deleted ID. Format 2 reuses the immutable base: one measured edit kept its 276647-byte base file unchanged and wrote a 528-byte checkpoint, then reopened with HEAD only.
- Read-only review findings are fixed: contiguous conflict paging, reserved IDs, generated-ID collisions including tombstones, omitted insert defaults, recoverable constraint failures, chunked durable conflict choices, retaining a newer known HEAD after publication replay, and the tile refresh handshake. Final reviewer reported no remaining blocking issue.
- Final actual spatial-engine host/signature tests 2/2 passed; all-target Clippy passed after the tile changes. The host returns its current random tile marker under the same authorization/publication transaction; preview rechecks the latest HEAD if another writer has advanced it.
- Final real-browser sparse insert/rebase/re-edit/publication preserved omitted defaults and explicit null. Replaying revision 11 after another user published revision 12 returned the current revision 12 tile marker.
- Cleanup complete: shared TaskSpace 50 finished once; fixture OPFS scopes and owned Vite listeners 5197/5198 cleaned; temporary Rust host 5199 stopped; auto-remove Docker container geoledger-collab-test-20261007 stopped. Only dedicated geoledger_test and mapseek_collab_browser_20261007 test environments were used; no existing business database was changed.
- GeoLedger/MapSeek remain local source dependencies; no crates.io upload, commit or push performed.

## RPC placement amendment

- User directed reuse of the existing featureserv Go microservice, with Thrift RPC to a private Rust runtime embedding GeoLedger. No separate Go service or Raft cluster is introduced.
- Gateway now calls FeatureServ.ExecuteCollaboration through its existing client. Featureserv validates the command/scope and calls DataVersionService.Execute; the Rust DatasetHost retains current permission and atomic publication checks.
- Old spatial-engine collaboration HTTP listener/entrypoint/path dependency removed. Runtime lives under services/featureserv/geoledger-runtime; configuration/startup and the collaboration guide moved accordingly. Local crate dependency remains in use.
- Gateway handler/router/config/cmd/wire checks passed; featureserv scoped application/client/RPC/wire/contract checks passed. Signatures bind original JSON and trusted scope; no automatic write retry. Existing feature response transport limits are preserved.
- GeoLedger scripts/check.sh passed again (docs, browser protocol, rustfmt, locked Clippy and workspace tests; optional database tests were not enabled for this invocation).
- Actual signed Kitex gateway → featureserv → Volo runtime → disposable PostGIS workflow passed (21.3 seconds, revisions 0→3): exact numeric snapshots, rejected actor/tenant/project, independent-field merge, same-field conflict preview and explicit resolution, publication replay without duplicate revisions. Final runtime build, six unit/transport tests, explicit PostGIS rollback/permission/marker test, rustfmt and all-target Clippy with warnings denied passed. The complete featureserv Go module also passed. The disposable runtime listener, database container and ephemeral signing keys were removed; no business database changed.

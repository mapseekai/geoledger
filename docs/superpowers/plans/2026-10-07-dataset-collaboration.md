# Dataset collaboration implementation plan

> Execute with superpowers:subagent-driven-development for the independent library and browser work, followed by cross-stack review. User approved the design and requested direct implementation; no additional design approval gate.

Goal: deliver the approved Dataset collaboration flow through an embedded geoledger-center crate.

Spec: [approved design](../specs/2026-10-07-mapseek-dataset-collaboration.md).

## Constraints

- Work on codex/dataset-collaboration in the two current checkouts; preserve unrelated user modifications.
- Development uses the local GeoLedger source dependency. Prepare release only after verification; never claim publication without registry evidence.
- Reuse core merge_record and existing center deadline-aware transactions. HTTP is an optional center feature.
- All versioned table writes, history, HEAD and host publication markers share one transaction.
- Tests use dedicated disposable databases and OPFS contexts. Keep current-format recovery and explicit format versions.

## Shared protocol

MapSeek exposes POST /v1/feature/collections/{collection}/collaboration. Body is a flat object tagged by op. Browser sends no identity; gateway signs trusted scope. Rust host resolves Dataset and physical table and authorizes within the library transaction. Revisions are decimal strings in JSON; workspace versions are bounded integers. Epoch and request_id are UUID strings. Feature IDs are strings.

| op | input beyond op | output |
| --- | --- | --- |
| register / head | none | epoch, revision, schema, id_column, geometry_column, srid |
| snapshot | epoch, revision, after?, limit? | epoch, revision, features, schema, next_after, done |
| changes | epoch, from, to, after?, limit? | epoch, from, to, changes, schema, next_after, done |
| open_draft | epoch, base_revision, request_id | workspace, version, base_revision |
| save_delta | epoch, workspace, expected_version, request_id, operations | workspace, version |
| preview | epoch, workspace, after?, limit? | head, version, conflicts, total, next_after |
| resolve | epoch, workspace, expected_version, expected_head, resolutions | workspace, version |
| rebase | epoch, workspace, expected_version, expected_head | workspace, version, base_revision |
| draft_changes | epoch, workspace, after?, limit? | base_revision, version, changes, schema, next_after, done |
| publish | epoch, workspace, expected_version, request_id, message? | workspace, version, revision, ids, status, changes |
| commit_result | epoch, request_id | original publication result or pending/unknown |
| history | epoch, after?, limit? | commits, next_after |
| commit | epoch, revision, after?, limit? | revision, changes, next_after, done |

schema is an array of {name,type,nullable,editable}; rows retain original PostgreSQL types on the server. changes contains {id,feature}, with explicit null for deletion. conflicts contains {id,fields,base,local,remote}. resolutions contains {id,choice} where choice is local/remote/custom; custom includes an explicit feature or null. Cursor binds the fixed interval in request parameters and server validation.

operations reuses the current editor batch shape: post with stable client_id and body Feature; patch with id and body containing changed properties and optional geometry; delete with id; patch with schema:true and body {add:[{name,type}],drop:[name]}. A schema addition/deletion is distinct from a null property. ids maps client_id to assigned permanent ID. No-ops do not manufacture commits.

Errors preserve HTTP status and JSON {error:{code,message},...details}; merge conflicts have status 409 and conflict fields at top level. Endpoints return raw successful JSON. Bounded chunks carry stable request IDs and hashes; retries must not change payloads. Message defaults to empty string.

## Task 1: embedded library and managed PostGIS history

Owner: library implementer. Files: center Cargo/lib/session/schema plus focused collaboration modules and integration tests.

- [ ] Add failing library tests for same-field conflicts, different-field merges, optional message and scoped cursor validation.
- [ ] Make server/bin dependencies optional and add typed CollaborationCommand, Scope, TableBinding and Host bridge contracts.
- [ ] Implement registration, fixed snapshots and changes, typed row/schema history, private incremental drafts, safe ID mapping, merge/rebase and idempotent publication.
- [ ] Expose CenterApplication::collaborate with a Host callback using the same deadline-aware transaction; native center behavior continues under the current explicit schema format.
- [ ] Test real PostgreSQL rollback, schema conflicts, large geometry, more than 1000 changes, request replay, and read isolation; run center tests and workspace checks where dependencies permit.

Host contract: authorize(tx, scope, write) resolves TableBinding and checks resource state; published(tx, scope, revision, schema_changed) updates host metadata/notification markers. Export Transaction for these trusted in-process callbacks. No SQL or host identity comes from browser body.

## Task 2: browser durable cache and sync client

Owner: cache implementer. Files: cloud features/dataset/lib/collaboration.ts and collaboration-cache.ts plus tests. Own these files only.

- [ ] Write tests for no-network full-data on warm open, stable snapshot target, deletion deltas, interrupted downloads, quota errors and retained pending edits.
- [ ] Implement typed protocol client and OPFS immutable page/generation manifest cache keyed by origin/user/tenant/project/dataset.
- [ ] Expose CollaborationSession.open(scope, source, signal), head/check, update, stage, preview, resolve, publish and history helpers; communicate exact exported types with UI implementer before integration.
- [ ] Support resumable first download and incremental updates, stable request IDs for every mutating operation and publication retry, schema changes and new IDs; use a lifetime Web Lock per working copy.
- [ ] Keep cached base distinct from working/pending and persist before acknowledging saves; run focused unit and real OPFS browser tests.

## Task 3: Dataset editor and conflict/history interface

Owner: editor implementer. Files: VectorEditorPage, focused collaboration UI components/tests; coordinate client interfaces with Task 2. Do not alter cache files.

- [ ] Test opening from cache, stale update prompt, empty comment, conflict choices, unknown publication result and leaving with retained draft.
- [ ] Reuse the current workbench and edit diff computation, use stable scope-specific OPFS project IDs, preserve current create-new-Dataset flow.
- [ ] Add status, update, optional comment/submit, history and conflict resolution UI with base/local/remote fields and geometry comparison.
- [ ] Freeze input or snapshot generation during publication. Preserve pending modifications across refresh and synchronization. Block silent force overwrites and stale resolutions.
- [ ] Run component tests, typecheck and targeted browser flow.

## Task 4: MapSeek host integration

Owner: controller. Files: spatial-engine collaboration module/main/Cargo, gateway feature handler/router plus client, featureserv managed-write guard and deployment/docs.

- [ ] Test signed request binding and query/update authorization, cross-scope denial, host publication rollback and old write rejection.
- [ ] Embed local center crate with default-features=false, mount internal collaboration route in the existing Rust process, verify existing Ed25519 request proof with distinct audience.
- [ ] Gateway checks existing Dataset permissions and signs trusted actor/tenant/project plus request bytes. Rust host resolves and locks Dataset publication rows inside library transaction.
- [ ] Publication advances Dataset content version and featureserv collection_changes atomically; old mutation APIs reject managed tables.
- [ ] Supply explicit schema initialization/setup and local library dependency instructions. Preserve existing transport/time limits and shutdown behavior.

## Task 5: integration, review and release preparation

- [ ] Reconcile shared contracts and test all three layers together with two identities and isolated PostGIS/OPFS data.
- [ ] Run documentation/browser protocol/fmt/Clippy/workspace tests, scoped MapSeek Go tests and cloud typecheck/tests. Distinguish pre-existing failures and environment limits.
- [ ] Obtain fresh review of commit atomicity, merge correctness, auth scoping, local persistence, retry and browser integration; fix material findings with covering regressions.
- [ ] Record measured first/warm/delta transfer and latency for actual fixture sizes; never infer scalability from unit tests.
- [ ] Package-check publishable crates and prepare version dependency switch only after verification. Registry upload requires valid credentials and a concrete verified release; no simulated publication.

## Review focus

- Property-only edits preserve exact original geometry and typed numeric values.
- Full snapshot pages never mix revisions; pending base remains valid after restart.
- Concurrent HEAD advance invalidates previously chosen conflict resolutions.
- Publication reply loss never causes duplicate inserts or commits.
- Cache identity and every database query enforce user/tenant/project/Dataset boundaries.

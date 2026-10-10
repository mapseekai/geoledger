# Managed Member Tokens Implementation Plan

> **For agentic workers:** Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Platform administrators issue, revoke and replace personal credentials with one-time plaintext disclosure.
**Architecture:** Application owns authorization and lifecycle; repository transactions persist digests, receipts and global audit. Transports authenticate managed credentials through Application, then use the existing business dispatch. Console calls typed SDK methods through BFF.
**Tech Stack:** Rust, SQLite/PostGIS, protobuf/gRPC, four SDKs, Next.js/React.
**Spec:** ../specs/2026-10-10-managed-member-tokens-design.md

## Global Constraints

- Format 12, fresh databases only; preserve existing local data and running service.
- No plaintext persistence, positive authentication cache, automatic issuance retries, or project-owner credential administration.
- File credentials remain operator-managed. JWT mode disables managed credentials.
- Native implementation in current feature branch; existing access-layout changes preserved. No automatic push.

## Review Focus

- Lost successful response: replay identifies created token but never rediscloses it (Task 1).
- Concurrent replacement: only one successor commits (Task 1).
- Administrator identity issuance and non-admin access rejected (Tasks 1–2).
- Revoked credential cannot reuse BFF cached reads or sessions (Tasks 2–3).
- Restore includes digests; logical exports exclude global credentials (Task 1).

## Task 1: Engine and storage

Files: engine/src/credentials.rs, repository.rs, session.rs, sqlite.sql, postgis.sql, lib.rs; engine/tests/credentials.rs.
Interface: operations list_tokens, issue_token, revoke_token, reissue_token; Application::authenticate_managed_token(&str) -> Result<Option<String>>. Metadata uses id, subject, label, created_at, expires_at, revoked_at, created_by. Issue returns credential metadata plus optional token and replayed boolean. Requests use request_id, subject, label, expires_at; reissue additionally token_id and derives subject from old token. Revoke uses token_id. Pages use after/limit.
- [x] Write lifecycle test using execute("admin", "issue_token", json!({...})); assert ordinary owner denied, replay.token absent, changed parameters rejected, then replacement invalidates old authentication.
- [x] Run cargo test -p geoledger-engine --test credentials; observe missing-operation failure.
- [x] Add transactional repository methods with global credential mutation lock; persist receipts without secret and global audit in same transaction. Generate 256+ random bits and SHA-256; validate expiry and administrator targets.
- [x] Implement format 12 in both schemas and update explicit current-format expectations.
- [x] Run lifecycle, concurrency, rollback and backup tests for both backends where available.

## Task 2: Transport and SDK

Files: proto/geoledger/v1/geoledger.proto, server/src/{lib,auth,http,rpc}.rs, sdk/{rust,go,python,ts}, cli/src.
Interface: ListTokens, IssueToken, RevokeToken, ReissueToken RPCs; Info gains platform_admin and managed_tokens booleans. Preserve snake_case in HTTP and language-native naming in SDKs.
- [x] Add server test: issue via admin HTTP, authenticate via new token, revoke, then Info returns 401; verify non-admin denied and JWT mode rejects issuance.
- [x] Make admission async; managed authentication runs in bounded blocking task with request timeout, no token fallback. Acquire capacity before database authentication.
- [x] Generate bindings with scripts/generate-sdk.sh, add typed convenience methods in four SDKs and CLI dispatch.
- [x] Run server integration and SDK tests, including response replay omission.

## Task 3: Console and session invalidation

Files: web/src/components/tokens.tsx, app/[section]/page.tsx, navigation, lib/{operations,requests,browser-api}.ts and auth/session call boundary.
- [x] Add BFF validation/dispatch tests for stable request_id, expiry and token ID.
- [x] Add platform-only navigation, paginated list, issue form, once-only dialog, revoke confirmation and reissue dialog; keep request_id during unknown outcome retry.
- [x] Set no-store on sensitive responses; validate managed credentials before serving cached protected data. Clear browser token state on dialog close and never persist it.
- [x] Run Web tests, typecheck, format, production build. Verify browser lifecycle on isolated format-12 service at separate ports; check desktop/mobile layout.

## Task 4: Documentation and acceptance

- [x] Document platform admin configuration, one-time disclosure and response loss, operator token exclusion, deployment format 12 and restore credential rotation.
- [x] Run ./scripts/check.sh with SpatiaLite and isolated PostGIS if available; SDK checks and backup drill.
- [x] Fresh review of complete diff, fix material issues, report measured checks and remaining limitations. Do not switch the user’s format-11 service automatically.

## Execution ledger

- User approved design and explicitly requested execution; proceed without a second planning permission gate.
- Feature branch zwishing/member-tokens; no worktree copy to avoid duplicating active local environment. Tests use isolated database directories.

### Progress

- Engine: issue/replay/revoke/reissue implemented; SQLite lifecycle, no-plaintext backup, failure rollback, expiry and concurrent reissue pass. PostGIS cross-instance race passes.
- Transport: typed RPCs plus shared HTTP/Application operations; managed admission holds capacity permit during database authentication; static-file fallback prohibited. JWT mode disabled.
- SDKs: regenerated pinned Go/Python/TS bindings; four business SDK methods added. Go/Python/TS existing suites pass; Rust server lifecycle passes.
- Web: platform-only token section, one-time dialog, stable request ID on retries, revoke/reissue confirmations; BFF no-store and managed session freshness. Web 60 tests, types and production build pass.
- Complete: full scripts/check.sh passed with SQLite and isolated PostGIS; browser flow, fresh review and docs verification complete.

- Browser: complete issue → grant viewer → member login → reissue → old BFF session 401 → new login → revoke → session 401; one-time plaintext cleared and desktop/mobile no overflow. Temporary plaintext file deleted.
- PostGIS: added failure-injection rollback, expiry and immutable audit/receipt checks; GL_TEST_PG_BIN-enabled native pg_dump/pg_restore passed.
- Fresh review: no blocking high/medium findings; reviewer noted PostGIS backup test gap, now covered by the explicit native backup run.

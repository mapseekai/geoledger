# Rust SDK error and cancellation contracts

Construct a client with `Client::connect(endpoint, api_key, ConnectOptions)`. This is the only connection entry point. `Client` is cloneable; clones share their connection. Connection setup has a ten-second bound and requests use the configured positive timeout (30 seconds by default). Plaintext remote endpoints require explicit opt-in. Drop the last client clone to release the channel.

Dropping a request future cancels local waiting, but does not establish that the server rolled back a write. Keep the `Workspace` after an uncertain publication. Its pending intent is set before the RPC: retry `Workspace::publish` with the same message, or persist `pending_publication()` and submit that exact `Publication` later. Do not change its request ID, version or payload. Refreshing a workspace does not clear a pending publication.

`publication_attempt_rejected` identifies a source-data rejection during one publication attempt before its commit. The current server returns HTTP 400 / gRPC `InvalidArgument`, which naturally produces `uncertain = false`. On the first `Workspace::publish` attempt, any definitive rejection clears pending intent, allowing correction and another publication. After an earlier unknown attempt, **only a validated business `workspace_version_conflict` with `uncertain = false` releases pending intent**. Every other retry error preserves the original publication, including generic/merge conflicts, source-data rejection, input, authentication, permission, invisible-resource and unrecognized business errors. A merge conflict can disappear when another workspace changes HEAD again; it does not prove the original attempt cannot commit. `workspace_version_conflict` specifically identifies the server's authoritative expected-workspace-version or closed-workspace check. An expected-HEAD mismatch is not this proof.

The SDK accepts an `ErrorDetail` as a business classification only when its code is nonempty and its enclosing `RpcStatus.code` matches the actual gRPC status. Bare or malformed `Aborted`, `AlreadyExists` and `FailedPrecondition` still map to `conflict`, with `uncertain = true`; a proxy may reject a retry while the original attempt is still running. Empty protobuf details and mismatched status envelopes cannot release a pending publication. A successful receipt retains the existing replay behavior. The lower-level `Client::publish(&Publication)` reports the uncertainty of its own attempt; callers preserving an earlier unknown attempt must retain that exact publication independently.

Undetailed gRPC `DeadlineExceeded` maps to `deadline_exceeded`. Resource-limit errors remain conservative because they do not prove that a write was rolled back.

## Per-message limits

`ConnectOptions::message_limits` sets separate outbound encoding and inbound decoding budgets through the same connection entry point as TLS and request timeouts.

```rust
use geoledger_client::{Client, ConnectOptions, MessageLimits};

async fn connect(endpoint: &str, api_key: &str) -> Result<Client, geoledger_client::Error> {
    let limits = MessageLimits::new(16 * 1024 * 1024, 64 * 1024 * 1024)?;
    Client::connect(
        endpoint,
        api_key,
        ConnectOptions {
            message_limits: limits,
            ..Default::default()
        },
    ).await
}
```

The first byte count limits requests and the second limits responses. Each must be in `1..=usize::MAX`; zero is rejected as `invalid_argument`. The counts cover a serialized Protobuf message and exclude its gRPC frame header. This SDK does not enable message compression. A message exactly at the configured limit is accepted; a larger message is rejected. Every client clone retains the same limits.

`MessageLimits::default()` uses **64 MiB in each direction**. Messages above that default are rejected unless the caller explicitly supplies a larger positive limit. gRPC framing and server request/response budgets also apply. These limits do not establish a strict process-memory ceiling because JSON conversion, encoding, decoded models, and concurrent requests can allocate additional memory.

Tonic's codec reports message-size limits with `OutOfRange`. An `OutOfRange` without a decoded GeoLedger business detail maps to `resource_exhausted`; an attached business detail keeps its own code. Either case remains `uncertain = true`, because the status does not establish whether a write completed or which peer rejected the message. The SDK does not infer a send/receive stage from error text. An outbound encoding failure can also be wrapped by the HTTP/2 transport and surface as `unavailable`, retaining `uncertain = true`. Follow the uncertainty and publication-recovery contract for these request failures; a specific resource error code is not guaranteed for outbound rejection.

If a publication reply exceeds the decoding budget, its immutable pending intent stays available and edits remain blocked. To recover with a larger budget, connect another client and submit the exact `Publication` from `pending_publication()`. A lower limit cannot establish that the server rejected the write. Increasing a client limit does not increase the server's limit.

This breaking API removes `connect_with_timeout`, `connect_with`, and `connect_with_message_limits`, and changes two-argument `connect` calls to pass `ConnectOptions`. Set `timeout` and `message_limits` in that value. Complete struct literals must include `message_limits`; `..Default::default()` selects the finite defaults. Deployments using messages above 64 MiB must choose larger explicit budgets.

The `gl` CLI uses the same 64 MiB defaults. Override them with `--max-encoding-bytes` / `GL_CLIENT_MAX_ENCODING_BYTES` and `--max-decoding-bytes` / `GL_CLIENT_MAX_DECODING_BYTES`. Zero is rejected. These are serialized Protobuf message byte counts excluding the gRPC frame header; server limits remain independent. For example, `gl --max-encoding-bytes 134217728 --max-decoding-bytes 134217728 info` selects 128 MiB per direction. These flags apply to every command, including `call`.

## Advanced business JSON

`Client::execute(operation, value)`, also used by the CLI, accepts an object containing only the current operation's request fields. Unknown fields, missing required fields and wrong JSON types return `invalid_argument` before an RPC is sent. `info` requires `{}`. Versioned writes require an explicit integer `expected_workspace_version`; resolve and rebase also require an integer `expected_head` and an explicit `resolutions` array. `archive_project` requires an explicit boolean `archived`, including `false` to reactivate.

Nested edit, schema-edit, added-field and PostGIS-table control fields are also strict. Edits require an explicit `feature`, with `null` representing deletion. GeoJSON `properties` remains an ordinary JSON object with arbitrary nested values and exact integers; property names are never interpreted as request fields. Server-side business, geometry, permission and transaction checks still apply.

Optional presence keeps its current meaning: omitted/null page limits select server defaults; feature-query options may be null and its default `bbox` is an empty array. For a save, omitted/null `schema_edits` preserves the existing intent and `[]` clears it. An omitted `edits` array remains a legal empty batch. Unknown or misspelled fields are not defaults.

## Diagnostic sources

The SDK derives `Display` and `std::error::Error` with `thiserror` and exposes business fields alongside a sanitized local source chain.

`std::error::Error::source()` exposes a sanitized diagnostic: JSON category and position, gRPC status code, or connection/TLS-configuration stage plus an underlying I/O error kind when available. Unknown opaque transport errors stay generic; the SDK does not guess DNS versus TLS from error strings. The original transport error and JSON payload text are deliberately not retained. The public business code/message remain separate from the diagnostic chain; server-provided business messages retain their existing semantics.

Cloning preserves diagnostic sources. Serialization keeps the existing five fields (`code`, `message`, `request_id`, `conflicts`, `uncertain`) and never includes sources; deserialization has no local source.

Construct errors with `Error::new(code, message)`, then set public optional fields or `uncertain` as needed. Locally generated JSON conversion errors use the generic public message `invalid JSON value` to avoid copying payload text; classification and location remain in the source.

```rust
use geoledger_client::Error;
let mut error = Error::new("unavailable", "service unavailable");
error.uncertain = true;
assert_eq!(error.code, "unavailable");
```

## Versioned schemas

`Client::schema(project, dataset, SchemaQuery)` reads fields at a published revision or workspace version. `Workspace::schema(dataset)` selects its own workspace. `Workspace::save_changes(edits, schema_edits)` atomically saves feature edits and the complete schema intent against the same optimistic version: `None` preserves the existing intent and `Some(&[])` clears it relative to the workspace baseline. `save_batch` keeps schema intent unchanged. Diff, commit and summary models expose `schema_changes` separately from feature changes. See the [API contract](../../docs/api.md#版本化字段定义与原子保存).

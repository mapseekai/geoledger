# Rust SDK error and cancellation contracts

`Client` is cloneable; clones share their connection. Connection setup has a ten-second bound and requests use the configured positive timeout (30 seconds by default). Plaintext remote endpoints require explicit opt-in. Drop the last client clone to release the channel.

Dropping a request future cancels local waiting, but does not establish that the server rolled back a write. Keep the `Workspace` after an uncertain publication. Its pending intent is set before the RPC: retry `Workspace::publish` with the same message, or persist `pending_publication()` and submit that exact `Publication` later. Do not change its request ID, version or payload. Refreshing a workspace does not clear a pending publication.

`publication_attempt_rejected` identifies a source-data rejection during one publication attempt before its commit. The server keeps HTTP 503 / gRPC `Unavailable` for this code, so older clients retain their conservative recovery behavior. On the first `Workspace::publish` attempt, the SDK clears pending intent and returns this error with `uncertain = false`, allowing the same workspace to save a correction and publish it. After an earlier uncertain attempt, this code preserves the original pending intent even if the new attempt's error has `uncertain = false`: an earlier execution may still finish. Input (`invalid_argument`) and authorization failures on a retry also preserve the intent. The lower-level `Client::publish(&Publication)` has no first-attempt context and retains the transport's uncertainty classification.

## Per-message limits

`Client::connect_with_message_limits(endpoint, api_key, ConnectOptions, MessageLimits)` sets separate outbound encoding and inbound decoding budgets. The existing `ConnectOptions` fields and struct-literal construction remain unchanged.

```rust
use geoledger_client::{Client, ConnectOptions, MessageLimits};

async fn connect(endpoint: &str, api_key: &str) -> Result<Client, geoledger_client::Error> {
    let limits = MessageLimits::new(16 * 1024 * 1024, 64 * 1024 * 1024)?;
    Client::connect_with_message_limits(
        endpoint,
        api_key,
        ConnectOptions::default(),
        limits,
    ).await
}
```

The first byte count limits requests and the second limits responses. Each must be in `1..=usize::MAX`; zero is rejected as `invalid_argument`. The counts cover a serialized Protobuf message and exclude its gRPC frame header. This SDK does not enable message compression. A message exactly at the configured limit is accepted; a larger message is rejected. Every client clone retains the same limits.

`MessageLimits::default()` uses `usize::MAX` in both directions. `connect`, `connect_with_timeout`, and `connect_with` continue to use that behavior, with no newly imposed SDK byte ceiling. gRPC framing and server request/response budgets still apply. Choose explicit budgets for your expected data sizes; these limits do not establish a strict process-memory ceiling because JSON conversion, encoding, decoded models, and concurrent requests can allocate additional memory.

Tonic's codec reports message-size limits with `OutOfRange`. An `OutOfRange` without a decoded GeoLedger business detail maps to `resource_exhausted`; an attached business detail keeps its own code. Either case remains `uncertain = true`, because the status does not establish whether a write completed or which peer rejected the message. This is a conservative change from the previous undetailed `OutOfRange` classification as `invalid_argument` with `uncertain = false`; the SDK does not infer a send/receive stage from error text. An outbound encoding failure can also be wrapped by the HTTP/2 transport and surface as `unavailable`, retaining `uncertain = true`. Follow the uncertainty and publication-recovery contract for these request failures; a specific resource error code is not guaranteed for outbound rejection.

If a publication reply exceeds the decoding budget, its immutable pending intent stays available and edits remain blocked. To recover with a larger budget, connect another client and submit the exact `Publication` from `pending_publication()`. A lower limit cannot establish that the server rejected the write. Increasing a client limit does not increase the server's limit.

## Advanced archive requests

The JSON entry point `Client::execute("archive_project", value)`, also used by the CLI, requires exactly a string `project` and a boolean `archived`. Missing `archived`, a misspelled field such as `archive`, unknown fields, and non-boolean values return `invalid_argument` before an archive RPC is sent. This intentionally rejects formerly tolerated malformed inputs. Explicit `archived: false` still reactivates a project, and the typed `Client::archive_project(project, archived)` API is unchanged. Other generated request/response defaults are unchanged.

## Diagnostic sources

The SDK derives `Display` and `std::error::Error` with `thiserror`; this keeps the public error fields, display text and sanitized source contract unchanged. Application errors are not exposed as `anyhow::Error`.

`std::error::Error::source()` exposes a sanitized diagnostic: JSON category and position, gRPC status code, or connection/TLS-configuration stage plus an underlying I/O error kind when available. Unknown opaque transport errors stay generic; the SDK does not guess DNS versus TLS from error strings. The original transport error and JSON payload text are deliberately not retained. The public business code/message remain separate from the diagnostic chain; server-provided business messages retain their existing semantics.

Cloning preserves diagnostic sources. Serialization keeps the existing five fields (`code`, `message`, `request_id`, `conflicts`, `uncertain`) and never includes sources; deserialization has no local source.

Adding a private diagnostic field is a **Rust source compatibility change for external Error struct literals**. Construct errors with `Error::new(code, message)`, then set public optional fields or `uncertain` as needed. Existing JSON error representations remain compatible. Locally generated JSON errors now use the generic public message `invalid JSON value` to avoid copying payload text; classification and location remain in the source.

```rust
use geoledger_client::Error;
let mut error = Error::new("unavailable", "service unavailable");
error.uncertain = true;
assert_eq!(error.code, "unavailable");
```

## Versioned schemas

`Client::schema(project, dataset, SchemaQuery)` reads fields at a published revision or workspace version. `Workspace::schema(dataset)` selects its own workspace. `Workspace::save_changes(edits, schema_edits)` atomically saves feature edits and the complete schema intent against the same optimistic version: `None` preserves the existing intent and `Some(&[])` clears it relative to the workspace baseline. `save_batch` keeps schema intent unchanged. Diff, commit and summary models expose `schema_changes` separately from feature changes. See the [API contract](../../docs/api.md#版本化字段定义与原子保存).

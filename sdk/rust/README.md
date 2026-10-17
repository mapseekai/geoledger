# Rust SDK error and cancellation contracts

`Client` is cloneable; clones share their connection. Connection setup has a ten-second bound and requests use the configured positive timeout (30 seconds by default). Plaintext remote endpoints require explicit opt-in. Drop the last client clone to release the channel.

Dropping a request future cancels local waiting, but does not establish that the server rolled back a write. Keep the `Workspace` after an uncertain publication. Its pending intent is set before the RPC: retry `Workspace::publish` with the same message, or persist `pending_publication()` and submit that exact `Publication` later. Do not change its request ID, version or payload. Refreshing a workspace does not clear a pending publication.

The SDK derives `Display` and `std::error::Error` with `thiserror`; this keeps the public error fields, display text and sanitized source contract unchanged. Application errors are not exposed as `anyhow::Error`.

## Diagnostic sources

`std::error::Error::source()` exposes a sanitized diagnostic: JSON category and position, gRPC status code, or connection/TLS-configuration stage plus an underlying I/O error kind when available. Unknown opaque transport errors stay generic; the SDK does not guess DNS versus TLS from error strings. The original transport error and JSON payload text are deliberately not retained. The public business code/message remain separate from the diagnostic chain; server-provided business messages retain their existing semantics.

Cloning preserves diagnostic sources. Serialization keeps the existing five fields (`code`, `message`, `request_id`, `conflicts`, `uncertain`) and never includes sources; deserialization has no local source.

Adding a private diagnostic field is a **Rust source compatibility change for external Error struct literals**. Construct errors with `Error::new(code, message)`, then set public optional fields or `uncertain` as needed. Existing JSON error representations remain compatible. Locally generated JSON errors now use the generic public message `invalid JSON value` to avoid copying payload text; classification and location remain in the source.

```rust
use geoledger_client::Error;
let mut error = Error::new("unavailable", "service unavailable");
error.uncertain = true;
assert_eq!(error.code, "unavailable");
```

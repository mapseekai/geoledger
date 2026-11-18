# GeoLedger Go SDK

The Go SDK uses API Keys and the current GeoLedger gRPC protocol. See [the API guide](../../docs/api.md) for connection and business examples. Upgrade the server and clients together for this release.

`Error.Code` uses `deadline_exceeded` for deadlines. Transport `OutOfRange` and `ResourceExhausted` use `resource_exhausted` and retain `Uncertain=true`; the status cannot establish whether a write committed. A structured business detail preserves its own code.

A source-data rejection returns `publication_attempt_rejected` over gRPC `InvalidArgument` (HTTP 400), with `Uncertain=false` for this attempt. On a first `Workspace.Publish`, the intent is released so the same workspace can save a correction. A rejection during recovery keeps the previously unknown publication's original request ID, version and payload, even though this attempt is definite. Authorization and input failures during recovery also keep the original intent. Only a trusted `workspace_version_conflict` can release a previously unknown intent; merge conflicts depend on the current HEAD and do not settle an older queued attempt.

Keep the workspace after cancellation, a lost response or an uncertain failure. Retry `Publish` with the original message, or persist `PendingPublication()` and submit that exact `Publication` through `Client.Publish`. Do not change the request ID or payload. Cancellation while waiting for the workspace's local lock sends no RPC and is certain; it does not resolve any earlier publication.

A conflict-family transport status without a valid current business detail remains uncertain. The detail envelope must agree with the transport status and contain a nonempty business code. Plain status text is not a workspace-version proof.

If recovery encounters a merge or another current-attempt rejection, preserve the intent. Use a separate current workspace handle to inspect and deliberately reconcile the draft with an atomic save, resolve or rebase that advances its version. Then retry the original intent: an earlier completed publication returns its receipt; otherwise the permanently stale expected version returns `workspace_version_conflict`. After that proof, refresh the original handle before editing or publishing a new intent. Recovery does not automatically discard an unknown request.

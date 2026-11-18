# GeoLedger Python SDK

The Python SDK uses API Keys and the current GeoLedger gRPC protocol. See [the API guide](../../docs/api.md) for business examples. Upgrade the server and clients together for this release.

`GeoLedgerError.code` uses `deadline_exceeded` for deadlines. Transport `OUT_OF_RANGE` and `RESOURCE_EXHAUSTED` use `resource_exhausted` with `uncertain=True`. These failures cannot establish that a write did not commit; a decoded business detail keeps its own code.

A source-data rejection returns `publication_attempt_rejected` over gRPC `INVALID_ARGUMENT` (HTTP 400), with `uncertain=False` for this attempt. A first `Workspace.publish` releases its intent, so the same workspace can save a correction. When an earlier attempt remains unknown, the same rejection preserves the original request ID, version and payload and keeps editing blocked. Authorization and input failures during recovery also preserve the intent. Only a trusted `workspace_version_conflict` can release a previously unknown intent; merge conflicts depend on the current HEAD and do not settle an older queued attempt.

After cancellation, a lost response or another uncertain failure, keep the workspace and retry `publish` with its original message. To recover in another process, persist `dataclasses.asdict(workspace.pending_publication)`, reconstruct the exact `Publication`, and submit it with `Client.publish`. Do not generate a new request ID for recovery. Python integer values preserve exact 64-bit GeoJSON property numbers.

A conflict-family transport status without a valid current business detail remains uncertain. The detail envelope must agree with the transport status and contain a nonempty business code. Plain status text is not a workspace-version proof.

If recovery encounters a merge or another current-attempt rejection, preserve the intent. Use a separate current workspace handle to inspect and deliberately reconcile the draft with an atomic save, resolve or rebase that advances its version. Then retry the original intent: an earlier completed publication returns its receipt; otherwise the permanently stale expected version returns `workspace_version_conflict`. After that proof, refresh the original handle before editing or publishing a new intent. Recovery does not automatically discard an unknown request.

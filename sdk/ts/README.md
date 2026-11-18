# GeoLedger Node.js SDK

This SDK uses API Keys and the current GeoLedger gRPC protocol. Import the public package entry point; browser applications use their BFF. See [the API guide](../../docs/api.md) for business examples. Upgrade the server and clients together for this release.

`GeoLedgerError.code` uses `deadline_exceeded` for deadlines. Transport `OutOfRange` and `ResourceExhausted` use `resource_exhausted` with `uncertain=true`; a codec or message-budget failure does not establish whether a write committed. Decoded business details keep their own code.

A source-data rejection returns `publication_attempt_rejected` over gRPC `InvalidArgument` (HTTP 400), with `uncertain=false` for this attempt. A first `Workspace.publish` releases the intent and allows correcting the saved draft. If an earlier attempt remains unknown, the same rejection keeps the original pending intent and blocks edits. Authorization and input failures during recovery also keep the intent. Only a trusted `workspace_version_conflict` can release a previously unknown intent; merge conflicts depend on the current HEAD and do not settle an older queued attempt.

After an uncertain failure, retry `Workspace.publish` with the original message. For recovery across processes, serialize `pendingPublication` with `stringifyJson`, retain every field, and restore its version as `BigInt` before calling `Client.publish`. Success replays the original immutable receipt. Preserve exact GeoJSON numbers with `parseJson` and `stringifyJson`.

A conflict-family transport status without a valid current business detail remains uncertain. The detail envelope must agree with the transport status and contain a nonempty business code. Plain status text is not a workspace-version proof.

If recovery encounters a merge or another current-attempt rejection, preserve the intent. Use a separate current workspace handle to inspect and deliberately reconcile the draft with an atomic save, resolve or rebase that advances its version. Then retry the original intent: an earlier completed publication returns its receipt; otherwise the permanently stale expected version returns `workspace_version_conflict`. After that proof, refresh the original handle before editing or publishing a new intent. Recovery does not automatically discard an unknown request.

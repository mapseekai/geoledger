import * as wire from "./geoledger/v1/geoledger";
import {
  parseJson,
  type Feature,
  type ServerInfo,
  type Project,
  type Dataset,
  type WorkspaceInfo,
  type SaveResult,
  type DiscardResult,
  type FeaturePage,
  type Change,
  type Diff,
  type Conflict,
  type Conflicts,
  type Commit,
  type CommitChanges,
  type AuditEvent,
  type AuditPage,
  type PublicationResult,
  type ResolutionResult,
  type RebaseResult,
  type Page,
  type FeatureQuery,
  type Edit,
  type Publication,
  type Json,
} from "../models";
function feature(value: wire.Feature | undefined): Feature | null {
  return value ? (parseJson(value.geojson) as unknown as Feature) : null;
}

export function decodeServerInfo(v: wire.InfoReply): ServerInfo {
  return {
    version: v.version,
    backend: v.backend,
    formatVersion: v.formatVersion,
    maxRequestBytes: v.maxRequestBytes,
    maxFeatureBytes: v.maxFeatureBytes,
  };
}
export function decodeProject(v: wire.ProjectReply): Project {
  return { id: v.project, name: v.name, head: BigInt(v.head), role: v.role };
}
export function decodeDataset(v: wire.DatasetReply): Dataset {
  return { id: v.dataset, name: v.name };
}
export function decodeWorkspaceInfo(v: wire.WorkspaceReply): WorkspaceInfo {
  return {
    id: v.workspace,
    baseRevision: BigInt(v.baseRevision),
    version: BigInt(v.version),
    status: v.status,
  };
}
export function decodeSaveResult(v: wire.SaveReply): SaveResult {
  return { version: BigInt(v.version), changes: BigInt(v.changes) };
}
export function decodeDiscardResult(v: wire.DiscardReply): DiscardResult {
  return { version: BigInt(v.version), status: v.status };
}
export function decodeFeaturePage(v: wire.FeaturesReply): FeaturePage {
  return {
    features: v.features.map((item) => feature(item) as Feature),
    revision: BigInt(v.revision),
    workspaceVersion:
      v.workspaceVersion === undefined ? undefined : BigInt(v.workspaceVersion),
    nextAfter: v.nextAfter === undefined ? undefined : v.nextAfter,
  };
}
export function decodeChange(v: wire.Change): Change {
  return {
    cursor: v.cursor,
    dataset: v.dataset,
    featureId: v.featureId,
    base: feature(v.base),
    draft: feature(v.draft),
    before: feature(v.before),
    after: feature(v.after),
  };
}
export function decodeDiff(v: wire.DiffReply): Diff {
  return {
    baseRevision: BigInt(v.baseRevision),
    version: BigInt(v.version),
    changes: v.changes.map((item) => decodeChange(item)),
  };
}
export function decodeConflict(v: wire.Conflict): Conflict {
  return {
    cursor: v.cursor,
    dataset: v.dataset,
    featureId: v.featureId,
    fields: v.fields.map((item) => item),
    base: feature(v.base),
    current: feature(v.current),
    draft: feature(v.draft),
    reason: v.reason,
    resolvedAgainstRevision:
      v.resolvedAgainstRevision === undefined
        ? undefined
        : BigInt(v.resolvedAgainstRevision),
  };
}
export function decodeConflicts(v: wire.ConflictsReply): Conflicts {
  return {
    head: BigInt(v.head),
    version: BigInt(v.version),
    total: BigInt(v.total),
    nextAfter: v.nextAfter === undefined ? undefined : v.nextAfter,
    truncated: v.truncated,
    conflicts: v.conflicts.map((item) => decodeConflict(item)),
  };
}
export function decodeCommit(v: wire.CommitInfo): Commit {
  return {
    revision: BigInt(v.revision),
    subject: v.subject,
    message: v.message,
    createdAt: v.createdAt,
  };
}
export function decodeCommitChanges(v: wire.CommitReply): CommitChanges {
  return {
    revision: BigInt(v.revision),
    changes: v.changes.map((item) => decodeChange(item)),
  };
}
export function decodeAuditEvent(v: wire.AuditEvent): AuditEvent {
  return {
    id: BigInt(v.id),
    subject: v.subject,
    action: v.action,
    detail: parseJson(v.detailJson),
    createdAt: v.createdAt,
  };
}
export function decodeAuditPage(v: wire.AuditReply): AuditPage {
  return {
    events: v.events.map((item) => decodeAuditEvent(item)),
    nextAfter: v.nextAfter === undefined ? undefined : BigInt(v.nextAfter),
  };
}
export function decodePublicationResult(
  v: wire.PublishReply,
): PublicationResult {
  return {
    revision: BigInt(v.revision),
    workspace: v.workspace,
    version: BigInt(v.version),
    status: v.status,
    changes: BigInt(v.changes),
  };
}
export function decodeResolutionResult(v: wire.ResolveReply): ResolutionResult {
  return {
    head: BigInt(v.head),
    version: BigInt(v.version),
    remainingConflicts: BigInt(v.remainingConflicts),
  };
}
export function decodeRebaseResult(v: wire.RebaseReply): RebaseResult {
  return {
    baseRevision: BigInt(v.baseRevision),
    version: BigInt(v.version),
    changes: BigInt(v.changes),
  };
}

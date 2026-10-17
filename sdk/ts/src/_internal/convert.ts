import * as wire from "./geoledger/v1/geoledger";
import {
  parseJson,
  type Feature,
  type ServerInfo,
  type Project,
  type Member,
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
  type HistoryPage,
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
function jsonObject(value: Json | undefined): value is { [key: string]: Json } {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}
function decodeFeature(value: wire.Feature): Feature {
  const parsed = parseJson(value.geojson);
  if (!jsonObject(parsed)) throw new Error("Invalid Feature object");
  const { id, properties, geometry } = parsed;
  if (
    parsed.type !== "Feature" ||
    typeof id !== "string" ||
    !jsonObject(properties) ||
    geometry === undefined ||
    (geometry !== null && !jsonObject(geometry))
  )
    throw new Error("Invalid Feature object");
  // Only check the transport envelope; exact properties and geometry stay untouched.
  return { ...parsed, type: "Feature", id, properties, geometry };
}
function feature(value: wire.Feature | undefined): Feature | null {
  return value ? decodeFeature(value) : null;
}

export function decodeServerInfo(v: wire.InfoReply): ServerInfo {
  return {
    platformAdmin: v.platformAdmin,
    version: v.version,
    backend: v.backend,
    formatVersion: v.formatVersion,
    maxRequestBytes: v.maxRequestBytes,
    maxFeatureBytes: v.maxFeatureBytes,
  };
}
export function decodeProject(v: wire.ProjectReply): Project {
  return {
    id: v.project,
    name: v.name,
    head: BigInt(v.head),
    role: v.role,
    state: v.state,
  };
}
export function decodeMember(v: wire.MemberInfo): Member {
  return { subject: v.subject, role: v.role };
}
export function decodeDataset(v: wire.DatasetReply): Dataset {
  const { geometryType, coordinateDimension } = v;
  if (
    geometryType !== "point" &&
    geometryType !== "line" &&
    geometryType !== "polygon"
  )
    throw new Error("Invalid dataset geometry type");
  if (coordinateDimension !== 2 && coordinateDimension !== 3)
    throw new Error("Invalid dataset coordinate dimension");
  return {
    ...(v.postgisTable ? { postgisTable: v.postgisTable } : {}),
    id: v.dataset,
    name: v.name,
    geometryType,
    coordinateDimension,
    crs: v.crs,
  };
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
  return {
    version: BigInt(v.version),
    changes: BigInt(v.changes),
    warnings: v.warnings,
  };
}
export function decodeDiscardResult(v: wire.DiscardReply): DiscardResult {
  return { version: BigInt(v.version), status: v.status };
}
export function decodeFeaturePage(v: wire.FeaturesReply): FeaturePage {
  return {
    crs: v.crs,
    features: v.features.map(decodeFeature),
    revision: BigInt(v.revision),
    ...(v.workspaceVersion === undefined
      ? {}
      : { workspaceVersion: BigInt(v.workspaceVersion) }),
    ...(v.nextAfter === undefined ? {} : { nextAfter: v.nextAfter }),
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
    ...(v.nextAfter === undefined ? {} : { nextAfter: v.nextAfter }),
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
    ...(v.resolvedAgainstRevision === undefined
      ? {}
      : { resolvedAgainstRevision: BigInt(v.resolvedAgainstRevision) }),
  };
}
export function decodeConflicts(v: wire.ConflictsReply): Conflicts {
  return {
    head: BigInt(v.head),
    version: BigInt(v.version),
    total: BigInt(v.total),
    ...(v.nextAfter === undefined ? {} : { nextAfter: v.nextAfter }),
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
    sourceWorkspace: v.sourceWorkspace,
    sourceBaseRevision: BigInt(v.sourceBaseRevision),
  };
}
export function decodeHistoryPage(v: wire.HistoryReply): HistoryPage {
  return {
    commits: v.commits.map(decodeCommit),
    ...(v.nextAfter === undefined ? {} : { nextAfter: BigInt(v.nextAfter) }),
  };
}
export function decodeCommitChanges(v: wire.CommitReply): CommitChanges {
  return {
    revision: BigInt(v.revision),
    changes: v.changes.map((item) => decodeChange(item)),
    ...(v.nextAfter === undefined ? {} : { nextAfter: v.nextAfter }),
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
    ...(v.nextAfter === undefined ? {} : { nextAfter: BigInt(v.nextAfter) }),
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

export function decodeChangeSummary(
  v: wire.ChangeSummaryReply,
): import("../models").ChangeSummary {
  const counts = (c: wire.ChangeCounts) => ({
    added: BigInt(c.added),
    deleted: BigInt(c.deleted),
    modified: BigInt(c.modified),
  });
  if (!v.total) throw new Error("Missing summary total");
  return {
    total: counts(v.total),
    datasets: v.datasets.map((d) => ({ id: d.id, name: d.name, ...counts(d) })),
    ...(v.version === undefined ? {} : { version: BigInt(v.version) }),
    ...(v.revision === undefined ? {} : { revision: BigInt(v.revision) }),
  };
}

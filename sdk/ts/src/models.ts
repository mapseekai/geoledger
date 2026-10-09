import { parse } from "lossless-json";
export type Json =
  null | boolean | string | number | bigint | Json[] | { [key: string]: Json };
export interface Feature {
  type: "Feature";
  id: string;
  properties: { [key: string]: Json };
  geometry: Json;
}
/** Parse JSON without silently rounding integral properties. */
export function parseJson(raw: string): Json {
  const protectedInput = protectProtoKeys(raw);
  const value = parse(protectedInput.raw, undefined, (token) => {
    const match = /^(-?)(\d+)(?:\.(\d+))?(?:[eE]([+-]?\d+))?$/.exec(token);
    if (!match) throw new Error("invalid JSON number");
    const digits = (match[2] + (match[3] ?? "")).replace(/^0+/, "");
    if (!digits) return token.startsWith("-") ? -0 : 0;
    const number = Number(token);
    if (!Number.isFinite(number) || number === 0)
      throw new Error("JSON number outside finite binary64 range");
    const scale = Number(match[4] ?? 0) - (match[3]?.length ?? 0);
    let integer: string | undefined;
    if (scale >= 0 && digits.length + scale <= 310)
      integer = digits + "0".repeat(scale);
    else if (
      scale < 0 &&
      -scale < digits.length &&
      digits
        .slice(scale)
        .split("")
        .every((c) => c === "0")
    )
      integer = digits.slice(0, scale);
    if (integer !== undefined) {
      const exact = BigInt((match[1] || "") + integer);
      return exact >= -9007199254740991n && exact <= 9007199254740991n
        ? Number(exact)
        : exact;
    }
    if (Number.isInteger(number) || Math.abs(number) >= Number.MAX_SAFE_INTEGER)
      throw new Error("fractional JSON number cannot be represented safely");
    return number;
  });
  return protectedInput.marker
    ? (restoreProtoKeys(value, protectedInput.marker) as Json)
    : (value as Json);
}

// lossless-json assigns into `{}`, so `__proto__` would invoke the inherited setter.
function protectProtoKeys(raw: string): { raw: string; marker?: string } {
  let marker = "\0geoledger_proto_key";
  for (let number = 0; raw.includes(JSON.stringify(marker)); number++)
    marker = `\0geoledger_proto_key_${number}`;
  let output = "";
  let changed = false;
  for (let index = 0; index < raw.length;) {
    if (raw[index] !== '"') {
      output += raw[index++];
      continue;
    }
    const start = index++;
    while (index < raw.length) {
      if (raw[index] === "\\") {
        index += 2;
        continue;
      }
      if (raw[index++] === '"') break;
    }
    const token = raw.slice(start, index);
    let after = index;
    while (/\s/.test(raw[after] ?? "")) after++;
    if (raw[after] === ":" && JSON.parse(token) === "__proto__") {
      output += JSON.stringify(marker);
      changed = true;
    } else output += token;
  }
  return changed ? { raw: output, marker } : { raw };
}

function restoreProtoKeys(value: unknown, marker: string): unknown {
  if (Array.isArray(value)) {
    for (let index = 0; index < value.length; index++)
      value[index] = restoreProtoKeys(value[index], marker);
    return value;
  }
  if (!value || typeof value !== "object") return value;
  const object = value as Record<string, unknown>;
  for (const key of Object.keys(object)) {
    const child = restoreProtoKeys(object[key], marker);
    if (key === marker) {
      Object.defineProperty(object, "__proto__", {
        value: child,
        enumerable: true,
        configurable: true,
        writable: true,
      });
      delete object[key];
    } else object[key] = child;
  }
  return object;
}

export interface ServerInfo {
  version: string;
  backend: string;
  formatVersion: number;
  maxRequestBytes: number;
  maxFeatureBytes: number;
}

export interface Project {
  id: string;
  name: string;
  head: bigint;
  role: string;
  /** `active`, `archived` (read-only) or `deleted`. */
  state: string;
}

export interface Member {
  subject: string;
  role: string;
}

export type GeometryType = "point" | "line" | "polygon";

export interface Dataset {
  geometryType: GeometryType;
  coordinateDimension: 2 | 3;
  id: string;
  name: string;
}

export interface WorkspaceInfo {
  id: string;
  baseRevision: bigint;
  version: bigint;
  status: string;
}

export interface SaveResult {
  warnings: string[];
  version: bigint;
  changes: bigint;
}

export interface DiscardResult {
  version: bigint;
  status: string;
}

export interface FeaturePage {
  features: Feature[];
  revision: bigint;
  workspaceVersion?: bigint;
  nextAfter?: string;
}

export interface Change {
  cursor: string;
  dataset: string;
  featureId: string;
  base: Feature | null;
  draft: Feature | null;
  before: Feature | null;
  after: Feature | null;
}

export interface Diff {
  baseRevision: bigint;
  version: bigint;
  changes: Change[];
}

export interface Conflict {
  cursor: string;
  dataset: string;
  featureId: string;
  fields: string[];
  base: Feature | null;
  current: Feature | null;
  draft: Feature | null;
  reason: string;
  resolvedAgainstRevision?: bigint;
}

export interface Conflicts {
  head: bigint;
  version: bigint;
  total: bigint;
  nextAfter?: string;
  truncated: boolean;
  conflicts: Conflict[];
}

export interface Commit {
  revision: bigint;
  subject: string;
  message: string;
  createdAt: string;
  sourceWorkspace: string;
  sourceBaseRevision: bigint;
}
export interface CommitChanges {
  revision: bigint;
  changes: Change[];
}

export interface AuditEvent {
  id: bigint;
  subject: string;
  action: string;
  detail: Json;
  createdAt: string;
}

export interface AuditPage {
  events: AuditEvent[];
  nextAfter?: bigint;
}

export interface PublicationResult {
  revision: bigint;
  workspace: string;
  version: bigint;
  status: string;
  changes: bigint;
}

export interface ResolutionResult {
  head: bigint;
  version: bigint;
  remainingConflicts: bigint;
}

export interface RebaseResult {
  baseRevision: bigint;
  version: bigint;
  changes: bigint;
}

export interface Page {
  after?: string;
  limit?: number;
}
export interface FeatureQuery extends Page {
  workspace?: string;
  revision?: bigint;
  featureId?: string;
  bbox?: number[];
}
export interface Edit {
  dataset: string;
  featureId: string;
  feature: Feature | string | null;
}
export interface Publication {
  readonly project: string;
  readonly workspace: string;
  readonly expectedWorkspaceVersion: bigint;
  readonly requestId: string;
  readonly message: string;
}

export type ChangeCounts = { added: bigint; deleted: bigint; modified: bigint };
export type ChangeSummary = {
  total: ChangeCounts;
  datasets: (ChangeCounts & { id: string; name: string })[];
  version?: bigint;
  revision?: bigint;
};

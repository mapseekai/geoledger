import { uncertainStatus } from "./http-status";
import { stringify } from "lossless-json";
import { parseLosslessJson } from "./conflicts";
import { featureItemsUrl } from "./feature-links";
export class ApiError extends Error {
  constructor(
    message: string,
    public status: number,
    public uncertain = false,
    public requestId?: string,
  ) {
    super(message);
  }
}
/** Caller cancellation preserves its reason; it does not imply a write was rolled back. */
export async function call<T>(
  payload: Record<string, unknown>,
  signal?: AbortSignal,
): Promise<T> {
  if (payload.action === "features")
    return send<T>(featureItemsUrl(payload), "GET", undefined, true, signal);
  return send<T>("/api/console", "POST", payload, false, signal);
}
export async function send<T>(
  path: string,
  method: string,
  payload?: unknown,
  featureResponse = false,
  signal?: AbortSignal,
): Promise<T> {
  const timeout = AbortSignal.timeout(45_000);
  const requestSignal = signal ? AbortSignal.any([signal, timeout]) : timeout;
  try {
    requestSignal.throwIfAborted();
    const response = await fetch(path, {
      method,
      headers: { "content-type": "application/json" },
      credentials: "same-origin",
      cache: "no-store",
      ...(payload === undefined ? {} : { body: JSON.stringify(payload) }),
      signal: requestSignal,
    });
    requestSignal.throwIfAborted();
    if (!response.ok) {
      if (
        response.status === 401 &&
        (path === "/api/console" ||
          path === "/api/auth" ||
          path.startsWith("/api/projects/"))
      ) {
        window.location.replace("/login?expired=1");
      }
      let detail: Record<string, unknown> = {};
      try {
        const value = JSON.parse(await response.text());
        if (value?.error && typeof value.error === "object")
          detail = value.error;
      } catch {
        // Gateway bodies may be empty, non-JSON or interrupted; the HTTP status is still known.
      }
      throw new ApiError(
        typeof detail.message === "string" ? detail.message : "请求失败",
        response.status,
        detail.uncertain === true || uncertainStatus(response.status),
        typeof detail.requestId === "string" ? detail.requestId : undefined,
      );
    }
    const raw = await response.text();
    requestSignal.throwIfAborted();
    const data = featureResponse
      ? (parseLosslessJson(raw) as Record<string, unknown>)
      : JSON.parse(raw);
    if (featureResponse) {
      if (data.type !== "FeatureCollection" || !Array.isArray(data.features))
        throw new Error("Invalid FeatureCollection");
      return {
        ...data,
        features: data.features.map((f: { id: string }) => ({
          id: String(f.id),
          geojson: stringify(f)!,
        })),
      } as T;
    }
    return data as T;
  } catch (error) {
    if (signal?.aborted && requestSignal.reason === signal.reason)
      throw signal.reason;
    if (error instanceof ApiError) throw error;
    throw new ApiError("连接中断或响应超时，请检查服务状态。", 0, true);
  }
}
export type Project = {
  id: string;
  name: string;
  head: string;
  role: string;
  /** active, archived (read-only) or deleted */
  state: string;
};
export type Member = { subject: string; role: string };
export type GeometryType = "point" | "line" | "polygon";
export type PostgisTable = {
  schema: string;
  table: string;
  idColumn: string;
  geometryColumn: string;
};
export type Dataset = {
  postgisTable?: PostgisTable;
  id: string;
  name: string;
  geometryType: GeometryType;
  coordinateDimension: 2 | 3;
  crs: string;
  crsDefinition?: string;
};
export type Workspace = {
  id: string;
  baseRevision: string;
  version: string;
  status: string;
};
export type Info = {
  platformAdmin: boolean;
  version: string;
  backend: string;
  formatVersion: number;
  maxFeatureBytes: number;
  maxRequestBytes: number;
};
export type Feature = { id: string; geojson: string };
export type FeaturePage = {
  features: Feature[];
  revision: string;
  workspaceVersion?: string;
  nextAfter?: string;
};
export type Commit = {
  revision: string;
  subject: string;
  message: string;
  createdAt: string;
  sourceWorkspace: string;
  sourceBaseRevision: string;
};
export type Audit = {
  id: string;
  subject: string;
  action: string;
  detail: string;
  createdAt: string;
};
export type Changes = {
  changes: { cursor: string; json: string }[];
  version?: string;
  revision?: string;
};
export type Conflicts = {
  conflicts: { cursor: string; json: string }[];
  head: string;
  version: string;
  total: string;
  nextAfter?: string;
  truncated: boolean;
};

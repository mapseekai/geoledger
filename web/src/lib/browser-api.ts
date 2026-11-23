import { invalidVersionProof, matchesPublicationReceipt } from "./publication";
import { errorEnvelope, uncertainStatus } from "./http-status";
import { stringify } from "lossless-json";
import { parseLosslessJson } from "./conflicts";
import { featureItemsUrl } from "./feature-links";
export class ApiError extends Error {
  constructor(
    message: string,
    public status: number,
    public uncertain = false,
    public requestId?: string,
    public code?: string,
  ) {
    super(message);
  }
}
async function readError(response: Response): Promise<ApiError> {
  let value: unknown;
  try {
    value = JSON.parse(await response.text());
  } catch {
    // Gateways can return a non-JSON body or interrupt it.
  }
  const { body, detail } = errorEnvelope(value);
  const { code, message, requestId, uncertain, conflicts } = detail;
  const { status } = response;
  const transportUncertain = uncertainStatus(status);
  if (
    typeof code !== "string" ||
    !code.trim() ||
    typeof message !== "string" ||
    typeof uncertain !== "boolean"
  ) {
    return new ApiError(
      "请求失败",
      status,
      transportUncertain ||
        status === 409 ||
        code === "workspace_version_conflict",
    );
  }
  return new ApiError(
    message,
    status,
    transportUncertain ||
      uncertain ||
      invalidVersionProof(
        status,
        code,
        message,
        requestId,
        body.conflicts === undefined ? conflicts : body.conflicts,
      ),
    typeof requestId === "string" ? requestId : undefined,
    code,
  );
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
      throw await readError(response);
    }
    const raw = await response.text();
    requestSignal.throwIfAborted();
    if (
      path === "/api/console" &&
      payload &&
      typeof payload === "object" &&
      "action" in payload &&
      payload.action === "publish"
    ) {
      const intent = payload as Record<string, unknown>;
      let receipt: unknown;
      try {
        receipt = JSON.parse(raw);
      } catch {
        /* Empty and malformed responses cannot confirm a publication. */
      }
      if (!matchesPublicationReceipt(receipt, intent.workspace, intent.version))
        throw new ApiError(
          "无法确认发布回执。",
          response.status,
          true,
          undefined,
          "invalid_response",
        );
      return receipt as T;
    }
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
export type HistoryPage = {
  commits: Commit[];
  nextAfter?: string;
};
export type Audit = {
  id: string;
  subject: string;
  action: string;
  detail: string;
  createdAt: string;
};
export type Changes = {
  nextAfter?: string;
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

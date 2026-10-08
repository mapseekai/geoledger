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
export async function call<T>(payload: Record<string, unknown>): Promise<T> {
  return send<T>("/api/console", "POST", payload);
}
export async function send<T>(
  path: string,
  method: string,
  payload?: unknown,
): Promise<T> {
  try {
    const response = await fetch(path, {
      method,
      headers: { "content-type": "application/json" },
      credentials: "same-origin",
      cache: "no-store",
      body: payload === undefined ? undefined : JSON.stringify(payload),
      signal: AbortSignal.timeout(45_000),
    });
    const data = await response.json();
    if (!response.ok) {
      if (response.status === 401 && path === "/api/console") {
        window.location.replace("/login?expired=1");
      }
      throw new ApiError(
        data.error?.message ?? "请求失败",
        response.status,
        data.error?.uncertain ?? response.status >= 500,
        data.error?.requestId,
      );
    }
    return data as T;
  } catch (error) {
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
  state?: string;
};
export type Member = { subject: string; role: string };
export type Dataset = { id: string; name: string };
export type Workspace = {
  id: string;
  baseRevision: string;
  version: string;
  status: string;
};
export type Info = {
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
};

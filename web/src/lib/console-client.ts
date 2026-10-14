import { httpCode, uncertainStatus } from "./http-status";
import type { Client, Edit, Conflicts } from "@geoledger/client";
import { stringifyJson, GeoLedgerError } from "@geoledger/client";
import { parseLosslessJson } from "./conflicts";
import { isLosslessNumber } from "lossless-json";
import { configuration } from "./config";

export type ConsoleClient = Pick<
  Client,
  | "info"
  | "projects"
  | "project"
  | "createProject"
  | "datasets"
  | "renameProject"
  | "renameDataset"
  | "deleteDataset"
  | "createDataset"
  | "trackTable"
  | "workspaces"
  | "workspaceInfo"
  | "workspaceSummary"
  | "commitSummary"
  | "createWorkspace"
  | "features"
  | "save"
  | "publish"
  | "diff"
  | "conflicts"
  | "resolve"
  | "rebase"
  | "discard"
  | "history"
  | "commit"
  | "restore"
  | "audit"
  | "members"
  | "archiveProject"
  | "deleteProject"
> & {
  setMember(
    project: string,
    subject: string,
    role: "owner" | "editor" | "viewer",
    currentPassword: string,
  ): Promise<void>;
  removeMember(
    project: string,
    subject: string,
    currentPassword: string,
  ): Promise<void>;
};

/** Auth JSON contains no business numeric values and is never exposed as a Session to the browser. */
export async function authRequest<T>(
  operation: string,
  input: unknown = {},
  sid?: string,
): Promise<T> {
  return request<T>("auth", operation, input, sid, false);
}
async function request<T>(
  area: "auth" | "api",
  operation: string,
  input: unknown,
  sid: string | undefined,
  business: boolean,
): Promise<T> {
  const response = await fetch(
    `${configuration().endpoint}/${area}/v1/${operation}`,
    {
      method: "POST",
      redirect: "error",
      cache: "no-store",
      headers: {
        "content-type": "application/json",
        ...(sid ? { "x-geoledger-session": sid } : {}),
      },
      body: stringifyJson(input),
      signal: AbortSignal.timeout(40_000),
    },
  );
  const raw = await response.text();
  if (!response.ok) {
    let value: Record<string, unknown> = {};
    try {
      const parsed = decode(parseLosslessJson(raw));
      if (parsed && typeof parsed === "object")
        value = parsed as Record<string, unknown>;
    } catch {
      // Proxies may reject admission with a plain-text response.
    }
    const detail =
      value.error && typeof value.error === "object"
        ? (value.error as Record<string, unknown>)
        : value;
    throw Object.assign(
      new GeoLedgerError(
        String(detail.code ?? httpCode(response.status)),
        String(
          detail.message ??
            (typeof value.error === "string" ? value.error : "请求失败"),
        ),
        response.headers.get("x-request-id") ??
          (typeof detail.requestId === "string" ? detail.requestId : undefined),
        Array.isArray(value.conflicts)
          ? (value as unknown as Conflicts)
          : undefined,
        uncertainStatus(response.status),
      ),
      { status: response.status },
    );
  }
  return (business ? decode(parseLosslessJson(raw)) : JSON.parse(raw)) as T;
}
/** Convert protocol metadata only; feature properties and audit detail remain exact JSON. */
function decode(value: unknown, key = ""): unknown {
  if (
    [
      "features",
      "base",
      "draft",
      "before",
      "after",
      "current",
      "detail",
      "properties",
      "geometry",
    ].includes(key)
  )
    return value;
  if (Array.isArray(value)) return value.map((v) => decode(v));
  if (isLosslessNumber(value)) {
    return [
      "head",
      "revision",
      "version",
      "base_revision",
      "workspace_version",
      "source_base_revision",
      "resolved_against_revision",
      "changes",
      "total",
      "added",
      "deleted",
      "modified",
      "remaining_conflicts",
      "id",
      "next_after",
    ].includes(key)
      ? BigInt(value.value)
      : Number(value.value);
  }
  if (!value || typeof value !== "object") return value;
  return Object.fromEntries(
    Object.entries(value).map(([name, v]) => [
      name.replace(/_([a-z])/g, (_, c: string) => c.toUpperCase()),
      decode(v, name),
    ]),
  );
}
function snake(input: Record<string, unknown>) {
  return Object.fromEntries(
    Object.entries(input)
      .filter(
        ([k, v]) =>
          v !== undefined && k !== "action" && k !== "workspaceVersion",
      )
      .map(([k, v]) => [k.replace(/[A-Z]/g, (c) => `_${c.toLowerCase()}`), v]),
  );
}
function edits(values: readonly Edit[] = []) {
  return values.map((value) => {
    let feature: unknown = value.feature;
    if (typeof feature === "string") {
      try {
        feature = parseLosslessJson(feature);
      } catch {
        throw new GeoLedgerError("invalid_argument", "GeoJSON 格式无效。");
      }
    }
    return { dataset: value.dataset, feature_id: value.featureId, feature };
  });
}
export function consoleClient(sid: string): ConsoleClient {
  const call = async <T>(
    op: string,
    input: Record<string, unknown> = {},
  ): Promise<T> => {
    const value = await request<
      Record<string, unknown> | Record<string, unknown>[]
    >("api", op, snake(input), sid, true);
    if (op === "features" && !Array.isArray(value)) {
      if (value.type === "Feature") {
        const { crs, revision, workspaceVersion, ...feature } = value;
        return {
          crs,
          revision,
          workspaceVersion: workspaceVersion ?? undefined,
          features: [feature],
        } as T;
      }
      value.workspaceVersion ??= undefined;
    }
    const identity = op.includes("project")
      ? "project"
      : op.includes("dataset")
        ? "dataset"
        : op.includes("workspace") || op === "restore"
          ? "workspace"
          : undefined;
    const rename = (row: Record<string, unknown>) =>
      identity && identity in row ? { ...row, id: row[identity] } : row;
    return (Array.isArray(value) ? value.map(rename) : rename(value)) as T;
  };
  return {
    info: () => call("info"),
    projects: (p = {}) => call("list_projects", { ...p }),
    project: (project) => call("get_project", { project }),
    createProject: (name) => call("create_project", { name }),
    datasets: (project, p = {}) => call("list_datasets", { project, ...p }),
    renameProject: (project, name) => call("rename_project", { project, name }),
    renameDataset: (project, dataset, name) =>
      call("rename_dataset", { project, dataset, name }),
    deleteDataset: async (project, dataset, confirmName) => {
      await call("delete_dataset", { project, dataset, confirmName });
    },
    createDataset: (
      project,
      name,
      geometryType,
      coordinateDimension = 2,
      crs = "EPSG:4326",
    ) =>
      call("create_dataset", {
        project,
        name,
        geometryType,
        coordinateDimension,
        crs,
      }),
    trackTable: (project, name, source) =>
      call("create_dataset", {
        project,
        name,
        postgisTable: snake({ ...source }),
      }),
    workspaces: (project, p = {}) => call("list_workspaces", { project, ...p }),
    workspaceInfo: (project, workspace) =>
      call("get_workspace", { project, workspace }),
    workspaceSummary: (project, workspace) =>
      call("workspace_summary", { project, workspace }),
    commitSummary: (project, revision) =>
      call("commit_summary", { project, revision }),
    createWorkspace: async (project) =>
      ({ info: await call("create_workspace", { project }) }) as Awaited<
        ReturnType<Client["createWorkspace"]>
      >,
    features: (project, dataset, p = {}) =>
      call("features", { ...p, project, dataset }),
    save: (project, workspace, expectedWorkspaceVersion, values) =>
      call("save", {
        project,
        workspace,
        expectedWorkspaceVersion,
        edits: edits(values),
      }),
    publish: (p) =>
      call("publish", {
        project: p.project,
        workspace: p.workspace,
        expectedWorkspaceVersion: p.expectedWorkspaceVersion,
        requestId: p.requestId,
        message: p.message,
      }),
    diff: (project, workspace, p = {}) =>
      call("diff", { project, workspace, ...p }),
    conflicts: (project, workspace, p = {}) =>
      call("conflicts", { project, workspace, ...p }),
    resolve: (
      project,
      workspace,
      expectedWorkspaceVersion,
      expectedHead,
      values,
    ) =>
      call("resolve", {
        project,
        workspace,
        expectedWorkspaceVersion,
        expectedHead,
        resolutions: edits(values),
      }),
    rebase: (
      project,
      workspace,
      expectedWorkspaceVersion,
      expectedHead,
      values,
    ) =>
      call("rebase", {
        project,
        workspace,
        expectedWorkspaceVersion,
        expectedHead,
        resolutions: edits(values),
      }),
    discard: (project, workspace, expectedWorkspaceVersion) =>
      call("discard", { project, workspace, expectedWorkspaceVersion }),
    history: (project, after = 0n, limit) =>
      call("history", { project, after, limit }),
    commit: (project, revision, p = {}) =>
      call("commit", { project, revision, after: p.after, limit: p.limit }),
    restore: async (project, revision) =>
      ({ info: await call("restore", { project, revision }) }) as Awaited<
        ReturnType<Client["restore"]>
      >,
    audit: (project, after = 0n, limit) =>
      call("audit", { project, after, limit }),
    setMember: async (project, subject, role, currentPassword) => {
      await call("set_member", { project, subject, role, currentPassword });
    },
    members: (project, p = {}) => call("list_members", { project, ...p }),
    removeMember: async (project, subject, currentPassword) => {
      await call("remove_member", { project, subject, currentPassword });
    },
    archiveProject: (project, archived) =>
      call("archive_project", { project, archived }),
    deleteProject: async (project, confirmName) => {
      await call("delete_project", { project, confirmName });
    },
  };
}

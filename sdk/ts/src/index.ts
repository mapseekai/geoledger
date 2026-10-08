import {
  credentials,
  InterceptingCall,
  Interceptor,
  ServiceError,
} from "@grpc/grpc-js";
import { randomUUID } from "node:crypto";
import { isIP } from "node:net";
import { stringify } from "lossless-json";
import * as wire from "./_internal/geoledger/v1/geoledger";
import * as model from "./models";
import * as convert from "./_internal/convert";
export type {
  Json,
  Feature,
  Project,
  Dataset,
  WorkspaceInfo,
  ServerInfo,
  SaveResult,
  DiscardResult,
  FeaturePage,
  Change,
  Diff,
  Conflict,
  Conflicts,
  Commit,
  CommitChanges,
  AuditEvent,
  AuditPage,
  PublicationResult,
  ResolutionResult,
  RebaseResult,
  Page,
  FeatureQuery,
  Edit,
  Publication,
} from "./models";
export { parseJson } from "./models";

export class GeoLedgerError extends Error {
  constructor(
    public readonly code: string,
    message: string,
    public readonly requestId?: string,
    public readonly conflicts?: model.Conflicts,
    public readonly uncertain = false,
  ) {
    super(message);
    this.name = "GeoLedgerError";
  }
}
const invalid = (message: string) =>
  new GeoLedgerError("invalid_argument", message);
function failure(error: ServiceError): GeoLedgerError {
  const uncertain = [1, 2, 4, 8, 13, 14, 15].includes(error.code);
  try {
    const raw = error.metadata.get("grpc-status-details-bin")[0];
    if (Buffer.isBuffer(raw)) {
      const detail = wire.RpcStatus.decode(raw).details.find(
        (d) => d.typeUrl === "type.googleapis.com/geoledger.v1.ErrorDetail",
      );
      if (detail) {
        const d = wire.ErrorDetail.decode(detail.value);
        return new GeoLedgerError(
          d.code,
          d.message,
          d.requestId,
          d.conflicts ? convert.decodeConflicts(d.conflicts) : undefined,
          uncertain,
        );
      }
    }
  } catch {
    /* Optional malformed details must not replace the original error. */
  }
  const codes: Record<number, string> = {
    1: "cancelled",
    3: "invalid_argument",
    4: "timeout",
    5: "not_found",
    6: "conflict",
    7: "permission_denied",
    8: "resource_exhausted",
    9: "conflict",
    10: "conflict",
    11: "invalid_argument",
    16: "unauthenticated",
  };
  return new GeoLedgerError(
    codes[error.code] || "unavailable",
    error.details || "request failed",
    undefined,
    undefined,
    uncertain,
  );
}
export function stringifyJson(value: unknown): string {
  const validate = (v: unknown, seen: Set<object>) => {
    if (
      typeof v === "number" &&
      (!Number.isFinite(v) || (Number.isInteger(v) && !Number.isSafeInteger(v)))
    )
      throw invalid(
        "use bigint or JSON text for integers beyond Number.MAX_SAFE_INTEGER",
      );
    if (v && typeof v === "object") {
      if (seen.has(v)) throw invalid("cyclic JSON is not supported");
      seen.add(v);
      for (const child of Object.values(v)) validate(child, seen);
      seen.delete(v);
    }
  };
  validate(value, new Set());
  const encoded = stringify(value);
  if (encoded === undefined) throw invalid("JSON value required");
  return encoded;
}
function edits(items: readonly model.Edit[]): wire.Edit[] {
  return items.map((e) => {
    if (!Object.hasOwn(e, "feature") || e.feature === undefined)
      throw invalid("edit requires an explicit feature; use null for deletion");
    let feature: wire.Feature | undefined;
    if (e.feature !== null) {
      const geojson =
        typeof e.feature === "string" ? e.feature : stringifyJson(e.feature);
      try {
        model.parseJson(geojson);
      } catch {
        throw invalid("invalid GeoJSON");
      }
      feature = { geojson };
    }
    return { dataset: e.dataset, featureId: e.featureId, feature };
  });
}
function revision(v: bigint): string {
  if (typeof v !== "bigint" || v < 0n || v > 9223372036854775807n)
    throw invalid("revision must be a nonnegative signed-64-bit bigint");
  return v.toString();
}
const page = (p: model.Page = {}) => ({
  after: p.after ?? "",
  limit: p.limit === undefined ? undefined : String(p.limit),
});

/** Node.js client. All methods return promises and ordinary business data. */
export class Client {
  #rpc: wire.GeoLedgerClient;
  #closed = false;
  /**
   * Plaintext `http://` is accepted only for loopback hosts unless `options.allowInsecure`
   * (or `GL_ALLOW_INSECURE_TRANSPORT=true`) is set: the bearer token would otherwise cross
   * the network unencrypted.
   */
  constructor(
    endpoint: string,
    token: string,
    timeoutMs = 30_000,
    options: ClientOptions = {},
  ) {
    let url: URL;
    try {
      url = new URL(endpoint);
    } catch {
      throw invalid("endpoint must be http(s)://host:port");
    }
    if (
      !["http:", "https:"].includes(url.protocol) ||
      !url.host ||
      url.pathname !== "/" ||
      url.search ||
      url.username ||
      url.password ||
      url.hash
    )
      throw invalid("endpoint must be http(s)://host:port");
    if (
      !token ||
      !/^[\x20-\x7e]+$/.test(token) ||
      !Number.isFinite(timeoutMs) ||
      timeoutMs <= 0
    )
      throw invalid("ASCII token and finite positive timeout required");
    if (
      url.protocol === "http:" &&
      !isLoopbackHost(url.hostname) &&
      !options.allowInsecure &&
      !["1", "true", "yes"].includes(process.env.GL_ALLOW_INSECURE_TRANSPORT ?? "")
    )
      throw invalid(
        "refusing to send credentials over plaintext http to a non-loopback host; use https or allowInsecure",
      );
    const auth: Interceptor = (options, nextCall) => {
      options.deadline = Date.now() + timeoutMs;
      return new InterceptingCall(nextCall(options), {
        start(metadata, listener, next) {
          metadata.set("authorization", `Bearer ${token}`);
          next(metadata, listener);
        },
      });
    };
    this.#rpc = new wire.GeoLedgerClient(
      url.host,
      url.protocol === "https:"
        ? credentials.createSsl()
        : credentials.createInsecure(),
      {
        interceptors: [auth],
        "grpc.max_receive_message_length": 4 * 1024 * 1024,
        "grpc.max_send_message_length": 4 * 1024 * 1024,
        "grpc.enable_retries": 0,
      },
    );
  }
  close(): void {
    this.#closed = true;
    this.#rpc.close();
  }
  #call<W, T>(
    send: (cb: (error: ServiceError | null, response?: W) => void) => void,
    decode: (reply: W) => T,
  ): Promise<T> {
    if (this.#closed) return Promise.reject(invalid("client is closed"));
    return new Promise((resolve, reject) => {
      try {
        send((error, reply) => {
          if (error) {
            reject(failure(error));
            return;
          }
          try {
            if (reply === undefined) throw new Error();
            resolve(decode(reply));
          } catch {
            reject(
              new GeoLedgerError(
                "invalid_response",
                "invalid data received from server",
                undefined,
                undefined,
                true,
              ),
            );
          }
        });
      } catch (e) {
        reject(e instanceof GeoLedgerError ? e : invalid("invalid request"));
      }
    });
  }
  info(): Promise<model.ServerInfo> {
    return this.#call((cb) => this.#rpc.info({}, cb), convert.decodeServerInfo);
  }
  createProject(name: string): Promise<model.Project> {
    return this.#call(
      (cb) => this.#rpc.createProject({ name }, cb),
      convert.decodeProject,
    );
  }
  project(project: string): Promise<model.Project> {
    return this.#call(
      (cb) => this.#rpc.getProject({ project }, cb),
      convert.decodeProject,
    );
  }
  projects(p: model.Page = {}): Promise<model.Project[]> {
    return this.#call<wire.ProjectsReply, model.Project[]>(
      (cb) => this.#rpc.listProjects(page(p), cb),
      (r) => r.projects.map(convert.decodeProject),
    );
  }
  async setMember(
    project: string,
    subject: string,
    role: string,
  ): Promise<void> {
    await this.#call(
      (cb) => this.#rpc.setMember({ project, subject, role }, cb),
      (r: wire.OkReply) => r.ok,
    );
  }
  createDataset(project: string, name: string): Promise<model.Dataset> {
    return this.#call(
      (cb) => this.#rpc.createDataset({ project, name }, cb),
      convert.decodeDataset,
    );
  }
  datasets(project: string, p: model.Page = {}): Promise<model.Dataset[]> {
    return this.#call<wire.DatasetsReply, model.Dataset[]>(
      (cb) => this.#rpc.listDatasets({ project, ...page(p) }, cb),
      (r) => r.datasets.map(convert.decodeDataset),
    );
  }
  async createWorkspace(project: string): Promise<Workspace> {
    const info = await this.#call(
      (cb) => this.#rpc.createWorkspace({ project }, cb),
      convert.decodeWorkspaceInfo,
    );
    return new Workspace(this, project, info);
  }
  async workspace(project: string, workspace: string): Promise<Workspace> {
    const info = await this.workspaceInfo(project, workspace);
    return new Workspace(this, project, info);
  }
  workspaceInfo(
    project: string,
    workspace: string,
  ): Promise<model.WorkspaceInfo> {
    return this.#call(
      (cb) => this.#rpc.getWorkspace({ project, workspace }, cb),
      convert.decodeWorkspaceInfo,
    );
  }
  workspaces(
    project: string,
    p: model.Page = {},
  ): Promise<model.WorkspaceInfo[]> {
    return this.#call<wire.WorkspacesReply, model.WorkspaceInfo[]>(
      (cb) => this.#rpc.listWorkspaces({ project, ...page(p) }, cb),
      (r) => r.workspaces.map(convert.decodeWorkspaceInfo),
    );
  }
  features(
    project: string,
    dataset: string,
    q: model.FeatureQuery = {},
  ): Promise<model.FeaturePage> {
    return this.#call(
      (cb) =>
        this.#rpc.features(
          {
            project,
            dataset,
            ...page(q),
            workspace: q.workspace,
            revision:
              q.revision === undefined ? undefined : revision(q.revision),
            featureId: q.featureId,
            bbox: q.bbox ?? [],
          },
          cb,
        ),
      convert.decodeFeaturePage,
    );
  }
  history(project: string, after = 0n, limit = 100): Promise<model.Commit[]> {
    return this.#call<wire.HistoryReply, model.Commit[]>(
      (cb) =>
        this.#rpc.history(
          { project, after: revision(after), limit: String(limit) },
          cb,
        ),
      (r) => r.commits.map(convert.decodeCommit),
    );
  }
  commit(
    project: string,
    version: bigint,
    p: model.Page = {},
  ): Promise<model.CommitChanges> {
    return this.#call(
      (cb) =>
        this.#rpc.commit(
          { project, revision: revision(version), ...page(p) },
          cb,
        ),
      convert.decodeCommitChanges,
    );
  }
  audit(project: string, after = 0n, limit = 100): Promise<model.AuditPage> {
    return this.#call(
      (cb) =>
        this.#rpc.audit(
          { project, after: revision(after), limit: String(limit) },
          cb,
        ),
      convert.decodeAuditPage,
    );
  }
  publish(intent: model.Publication): Promise<model.PublicationResult> {
    return this.#call(
      (cb) =>
        this.#rpc.publish(
          {
            ...intent,
            expectedWorkspaceVersion: revision(intent.expectedWorkspaceVersion),
          },
          cb,
        ),
      convert.decodePublicationResult,
    );
  }
  async restore(project: string, version: bigint): Promise<Workspace> {
    const info = await this.#call(
      (cb) => this.#rpc.restore({ project, revision: revision(version) }, cb),
      convert.decodeWorkspaceInfo,
    );
    return new Workspace(this, project, info);
  }
  save(
    project: string,
    workspace: string,
    version: bigint,
    items: readonly model.Edit[],
  ): Promise<model.SaveResult> {
    return this.#call(
      (cb) =>
        this.#rpc.save(
          {
            project,
            workspace,
            expectedWorkspaceVersion: revision(version),
            edits: edits(items),
          },
          cb,
        ),
      convert.decodeSaveResult,
    );
  }
  diff(
    project: string,
    workspace: string,
    p: model.Page = {},
  ): Promise<model.Diff> {
    return this.#call(
      (cb) => this.#rpc.diff({ project, workspace, ...page(p) }, cb),
      convert.decodeDiff,
    );
  }
  conflicts(
    project: string,
    workspace: string,
    p: model.Page = {},
  ): Promise<model.Conflicts> {
    return this.#call(
      (cb) => this.#rpc.conflicts({ project, workspace, ...page(p) }, cb),
      convert.decodeConflicts,
    );
  }
  resolve(
    project: string,
    workspace: string,
    version: bigint,
    head: bigint,
    items: readonly model.Edit[],
  ): Promise<model.ResolutionResult> {
    return this.#call(
      (cb) =>
        this.#rpc.resolve(
          {
            project,
            workspace,
            expectedWorkspaceVersion: revision(version),
            expectedHead: revision(head),
            resolutions: edits(items),
          },
          cb,
        ),
      convert.decodeResolutionResult,
    );
  }
  rebase(
    project: string,
    workspace: string,
    version: bigint,
    head: bigint,
    items: readonly model.Edit[] = [],
  ): Promise<model.RebaseResult> {
    return this.#call(
      (cb) =>
        this.#rpc.rebase(
          {
            project,
            workspace,
            expectedWorkspaceVersion: revision(version),
            expectedHead: revision(head),
            resolutions: edits(items),
          },
          cb,
        ),
      convert.decodeRebaseResult,
    );
  }
  discard(
    project: string,
    workspace: string,
    version: bigint,
  ): Promise<model.DiscardResult> {
    return this.#call(
      (cb) =>
        this.#rpc.discard(
          { project, workspace, expectedWorkspaceVersion: revision(version) },
          cb,
        ),
      convert.decodeDiscardResult,
    );
  }
}
export interface ClientOptions {
  /** Permit plaintext http:// to a non-loopback host (trusted private networks only). */
  allowInsecure?: boolean;
}
/** True for `localhost` and loopback IP literals. */
export function isLoopbackHost(host: string): boolean {
  const h = host.replace(/^\[|\]$/g, "").toLowerCase();
  if (h === "localhost" || h === "::1") return true;
  if (isIP(h) === 4) return h.startsWith("127.");
  return false;
}
export function connect(
  endpoint: string,
  token: string,
  timeoutMs = 30_000,
  options: ClientOptions = {},
): Client {
  return new Client(endpoint, token, timeoutMs, options);
}

export class Workspace {
  #info: model.WorkspaceInfo;
  #pending?: model.Publication;
  #busy = false;
  constructor(
    private readonly client: Client,
    private readonly project: string,
    info: model.WorkspaceInfo,
  ) {
    this.#info = Object.freeze({ ...info });
  }
  get id(): string {
    return this.#info.id;
  }
  get info(): Readonly<model.WorkspaceInfo> {
    return this.#info;
  }
  get pendingPublication(): model.Publication | undefined {
    return this.#pending;
  }
  #editable(): void {
    if (this.#pending)
      throw invalid(
        "retry the pending publication before changing this workspace",
      );
    if (this.#info.status !== "open") throw invalid("workspace is closed");
  }
  #update<
    T extends { version?: bigint; status?: string; baseRevision?: bigint },
  >(r: T): T {
    this.#info = Object.freeze({
      ...this.#info,
      ...(r.version === undefined ? {} : { version: r.version }),
      ...(r.status === undefined ? {} : { status: r.status }),
      ...(r.baseRevision === undefined ? {} : { baseRevision: r.baseRevision }),
    });
    return r;
  }
  async #exclusive<T>(fn: () => Promise<T>): Promise<T> {
    if (this.#busy)
      throw invalid(
        "workspace operation already in progress; await it before starting another",
      );
    this.#busy = true;
    try {
      return await fn();
    } finally {
      this.#busy = false;
    }
  }
  async refresh(): Promise<Readonly<model.WorkspaceInfo>> {
    return this.#exclusive(async () => {
      this.#info = Object.freeze(
        await this.client.workspaceInfo(this.project, this.id),
      );
      return this.info;
    });
  }
  async save(
    dataset: string,
    feature: model.Feature | string,
  ): Promise<model.SaveResult> {
    let parsed;
    try {
      parsed = typeof feature === "string" ? model.parseJson(feature) : feature;
    } catch {
      throw invalid("invalid GeoJSON");
    }
    if (
      !parsed ||
      typeof parsed !== "object" ||
      !("id" in parsed) ||
      typeof parsed.id !== "string"
    )
      throw invalid("GeoJSON Feature requires a string id");
    return this.saveBatch([{ dataset, featureId: parsed.id, feature }]);
  }
  delete(dataset: string, featureId: string): Promise<model.SaveResult> {
    return this.saveBatch([{ dataset, featureId, feature: null }]);
  }
  saveBatch(items: readonly model.Edit[]): Promise<model.SaveResult> {
    return this.#exclusive(async () => {
      this.#editable();
      return this.#update(
        await this.client.save(this.project, this.id, this.info.version, items),
      );
    });
  }
  features(
    dataset: string,
    q: Omit<model.FeatureQuery, "workspace" | "revision"> = {},
  ): Promise<model.FeaturePage> {
    if ("workspace" in q || "revision" in q)
      return Promise.reject(
        invalid("draft queries cannot override workspace or revision"),
      );
    return this.client.features(this.project, dataset, {
      ...q,
      workspace: this.id,
    });
  }
  diff(p: model.Page = {}): Promise<model.Diff> {
    return this.client.diff(this.project, this.id, p);
  }
  conflicts(p: model.Page = {}): Promise<model.Conflicts> {
    return this.client.conflicts(this.project, this.id, p);
  }
  publish(message: string): Promise<model.PublicationResult> {
    return this.#exclusive(async () => {
      const wasPending = this.#pending !== undefined;
      if (!this.#pending) {
        this.#editable();
        this.#pending = Object.freeze({
          project: this.project,
          workspace: this.id,
          expectedWorkspaceVersion: this.info.version,
          requestId: randomUUID(),
          message,
        });
      } else if (this.#pending.message !== message)
        throw invalid(
          "retry the pending publication with the original message",
        );
      try {
        return this.#update(await this.client.publish(this.#pending));
      } catch (e) {
        if (
          e instanceof GeoLedgerError &&
          !e.uncertain &&
          (!wasPending ||
            !["unauthenticated", "permission_denied", "not_found"].includes(e.code))
        )
          this.#pending = undefined;
        throw e;
      }
    });
  }
  resolve(
    head: bigint,
    items: readonly model.Edit[],
  ): Promise<model.ResolutionResult> {
    return this.#exclusive(async () => {
      this.#editable();
      return this.#update(
        await this.client.resolve(
          this.project,
          this.id,
          this.info.version,
          head,
          items,
        ),
      );
    });
  }
  rebase(
    head: bigint,
    items: readonly model.Edit[] = [],
  ): Promise<model.RebaseResult> {
    return this.#exclusive(async () => {
      this.#editable();
      return this.#update(
        await this.client.rebase(
          this.project,
          this.id,
          this.info.version,
          head,
          items,
        ),
      );
    });
  }
  discard(): Promise<model.DiscardResult> {
    return this.#exclusive(async () => {
      this.#editable();
      return this.#update(
        await this.client.discard(this.project, this.id, this.info.version),
      );
    });
  }
}

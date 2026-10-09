import { Client, GeoLedgerError, stringifyJson } from "@geoledger/client";
import { requestSchema } from "./requests";
import { summarizeChanges, SummaryVersionError } from "./change-summary";
/** The only bridge to the business SDK. No generated transport types or forwarding. */
export async function execute(
  client: Client,
  input: unknown,
): Promise<unknown> {
  const r = requestSchema.parse(input);
  switch (r.action) {
    case "info":
      return client.info();
    case "projects":
      return client.projects(r);
    case "project":
      return client.project(r.project);
    case "createProject":
      return client.createProject(r.name);
    case "datasets":
      return client.datasets(r.project, r);
    case "renameProject":
      return client.renameProject(r.project, r.name);
    case "renameDataset":
      return client.renameDataset(r.project, r.dataset, r.name);
    case "deleteDataset":
      await client.deleteDataset(r.project, r.dataset, r.confirmName);
      return { ok: true };
    case "createDataset":
      return client.createDataset(r.project, r.name, r.geometryType);
    case "workspaces":
      return client.workspaces(r.project, r);
    case "workspace":
      return client.workspaceInfo(r.project, r.workspace);
    case "workspaceSummary":
    case "commitSummary": {
      const names = new Map<string, string>();
      let after = "";
      for (;;) {
        const datasets = await client.datasets(r.project, {
          after,
          limit: 100,
        });
        for (const dataset of datasets) names.set(dataset.id, dataset.name);
        if (datasets.length < 100) break;
        after = datasets.at(-1)!.id;
      }
      if (r.action === "commitSummary")
        return summarizeChanges(
          "commit",
          (after, limit) =>
            client.commit(r.project, BigInt(r.revision), { after, limit }),
          names,
        );
      const workspace = await client.workspaceInfo(r.project, r.workspace);
      const result = await summarizeChanges(
        "workspace",
        (after, limit) => client.diff(r.project, r.workspace, { after, limit }),
        names,
        workspace.version,
      ).catch((error: unknown) => {
        if (error instanceof SummaryVersionError)
          throw new GeoLedgerError("conflict", error.message);
        throw error;
      });
      const current = await client.workspaceInfo(r.project, r.workspace);
      if (current.version !== workspace.version)
        throw new GeoLedgerError(
          "conflict",
          "工作区已变化，请刷新后重新统计。",
        );
      return result;
    }
    case "createWorkspace":
      return (await client.createWorkspace(r.project)).info;
    case "features": {
      const result = await client.features(r.project, r.dataset, {
        ...r,
        revision: r.revision === undefined ? undefined : BigInt(r.revision),
      });
      // Keep GeoJSON text exact across the browser boundary, including uint64 properties.
      return {
        ...result,
        features: result.features.map((f) => ({
          id: f.id,
          geojson: stringifyJson(f),
        })),
      };
    }
    case "save":
      return client.save(r.project, r.workspace, BigInt(r.version), r.edits);
    case "publish":
      return client.publish({
        ...r,
        expectedWorkspaceVersion: BigInt(r.version),
      });
    case "diff": {
      const result = await client.diff(r.project, r.workspace, r);
      return {
        ...result,
        changes: result.changes.map((c) => ({
          cursor: c.cursor,
          json: stringifyJson(c),
        })),
      };
    }
    case "conflicts": {
      const result = await client.conflicts(r.project, r.workspace, r);
      return {
        ...result,
        conflicts: result.conflicts.map((c) => ({
          cursor: c.cursor,
          json: stringifyJson(c),
        })),
      };
    }
    case "resolve":
      return client.resolve(
        r.project,
        r.workspace,
        BigInt(r.version),
        BigInt(r.head),
        r.edits,
      );
    case "rebase":
      return client.rebase(
        r.project,
        r.workspace,
        BigInt(r.version),
        BigInt(r.head),
        r.edits,
      );
    case "discard":
      return client.discard(r.project, r.workspace, BigInt(r.version));
    case "history":
      return client.history(r.project, BigInt(r.after ?? "0"), r.limit);
    case "commit": {
      const result = await client.commit(r.project, BigInt(r.revision), r);
      return {
        ...result,
        changes: result.changes.map((c) => ({
          cursor: c.cursor,
          json: stringifyJson(c),
        })),
      };
    }
    case "restore":
      return (await client.restore(r.project, BigInt(r.revision))).info;
    case "audit": {
      const result = await client.audit(
        r.project,
        BigInt(r.after ?? "0"),
        r.limit,
      );
      return {
        ...result,
        events: result.events.map((e) => ({
          ...e,
          detail: stringifyJson(e.detail),
        })),
      };
    }
    case "setMember":
      await client.setMember(r.project, r.subject, r.role);
      return { ok: true };
    case "members":
      return client.members(r.project, r);
    case "removeMember":
      await client.removeMember(r.project, r.subject);
      return { ok: true };
    case "archiveProject":
      return client.archiveProject(r.project, r.archived);
    case "deleteProject":
      await client.deleteProject(r.project, r.confirmName);
      return { ok: true };
  }
}
export function encode(value: unknown) {
  return JSON.stringify(value, (_, item) =>
    typeof item === "bigint" ? item.toString() : item,
  );
}

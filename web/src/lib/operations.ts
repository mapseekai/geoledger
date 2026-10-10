import { Client, stringifyJson } from "@geoledger/client";
import { featureCollection } from "./feature-collection";
import { requestSchema } from "./requests";
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
      return client.createDataset(
        r.project,
        r.name,
        r.geometryType,
        r.coordinateDimension,
      );
    case "attachPostgisTable":
      return client.attachPostgisTable(r.project, r.name, r.source);
    case "workspaces":
      return client.workspaces(r.project, r);
    case "workspace":
      return client.workspaceInfo(r.project, r.workspace);
    case "workspaceSummary":
      return client.workspaceSummary(r.project, r.workspace);
    case "commitSummary":
      return client.commitSummary(r.project, BigInt(r.revision));
    case "createWorkspace":
      return (await client.createWorkspace(r.project)).info;
    case "features":
      return featureCollection(client, r);
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

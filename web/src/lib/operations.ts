import { Client, stringifyJson } from "@geoledger/client";
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
    case "createDataset":
      return client.createDataset(r.project, r.name);
    case "workspaces":
      return client.workspaces(r.project, r);
    case "workspace":
      return client.workspaceInfo(r.project, r.workspace);
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
  }
}
export function encode(value: unknown) {
  return JSON.stringify(value, (_, item) =>
    typeof item === "bigint" ? item.toString() : item,
  );
}

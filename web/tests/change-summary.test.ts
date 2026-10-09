import assert from "node:assert/strict";
import test from "node:test";
import { summarizeChanges } from "../src/lib/change-summary";
import { execute } from "../src/lib/operations";
import type { Client } from "@geoledger/client";

const feature = { type: "Feature" };
const change = (i: number, dataset = "roads") => ({
  cursor: String(i),
  dataset,
  base: null,
  draft: feature,
  before: null,
  after: feature,
});

test("counts all pages per dataset and counts a changed feature once", async () => {
  const rows = Array.from({ length: 23 }, (_, i) => change(i));
  const calls: string[] = [];
  const result = await summarizeChanges(
    "workspace",
    async (after, limit) => {
      calls.push(after);
      const offset = after ? Number(after) + 1 : 0;
      return { version: 2n, changes: rows.slice(offset, offset + limit) };
    },
    new Map([["roads", "道路"]]),
    2n,
  );
  assert.deepEqual(calls, ["", "19"]);
  assert.deepEqual(result, {
    total: { added: 23, deleted: 0, modified: 0 },
    datasets: [
      { id: "roads", name: "道路", added: 23, deleted: 0, modified: 0 },
    ],
  });
});

test("commit counts use before/after and keep datasets separate", async () => {
  const result = await summarizeChanges(
    "commit",
    async () => ({
      changes: [
        change(0),
        { ...change(1), before: feature, after: null },
        { ...change(2, "buildings"), before: feature, after: feature },
        { ...change(3), before: null, after: null },
      ],
    }),
    new Map([
      ["roads", "道路"],
      ["buildings", "建筑"],
    ]),
  );
  assert.deepEqual(result.total, { added: 1, deleted: 1, modified: 1 });
  assert.deepEqual(result.datasets, [
    { id: "roads", name: "道路", added: 1, deleted: 1, modified: 0 },
    { id: "buildings", name: "建筑", added: 0, deleted: 0, modified: 1 },
  ]);
});

test("shrinks oversized pages and rejects inconsistent workspace versions", async () => {
  const limits: number[] = [];
  const result = await summarizeChanges(
    "workspace",
    async (_, limit) => {
      limits.push(limit);
      if (limit > 5)
        throw Object.assign(new Error("too large"), {
          code: "resource_exhausted",
        });
      return { version: 1n, changes: [change(0)] };
    },
    new Map(),
    1n,
  );
  assert.deepEqual(limits, [20, 10, 5]);
  assert.equal(result.total.added, 1);
  await assert.rejects(
    summarizeChanges(
      "workspace",
      async () => ({ version: 2n, changes: [] }),
      new Map(),
      1n,
    ),
    /工作区已变化/,
  );
});

test("summary BFF resolves dataset names and rechecks workspace version", async () => {
  let reads = 0;
  const client = {
    datasets: async () => [{ id: "roads", name: "道路" }],
    workspaceInfo: async () => ({ version: ++reads === 1 ? 1n : 2n }),
    diff: async () => ({ version: 1n, changes: [change(0)] }),
  } as unknown as Client;
  await assert.rejects(
    execute(client, {
      action: "workspaceSummary",
      project: "p",
      workspace: "w",
    }),
    /工作区已变化/,
  );
  assert.equal(reads, 2);
});

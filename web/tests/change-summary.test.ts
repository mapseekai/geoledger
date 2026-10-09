import assert from "node:assert/strict";
import test from "node:test";
import { sharedRequests } from "../src/lib/change-summary";
import { execute } from "../src/lib/operations";
import type { Client } from "@geoledger/client";

test("summary uses one aggregate call without loading names or feature bodies", async () => {
  const calls: unknown[] = [];
  const result = {
    total: { added: 9384n, deleted: 0n, modified: 0n },
    datasets: [
      { id: "d", name: "LUCC", added: 9384n, deleted: 0n, modified: 0n },
    ],
    version: 503n,
  };
  const client = {
    workspaceSummary: async (...args: unknown[]) => {
      calls.push(args);
      return result;
    },
    commitSummary: async (...args: unknown[]) => {
      calls.push(args);
      return { ...result, version: undefined, revision: 1n };
    },
    diff: () => {
      throw new Error("must not fetch geometry");
    },
    commit: () => {
      throw new Error("must not fetch geometry");
    },
    datasets: () => {
      throw new Error("names must share summary snapshot");
    },
  } as unknown as Client;
  assert.equal(
    await execute(client, {
      action: "workspaceSummary",
      project: "p",
      workspace: "w",
    }),
    result,
  );
  const commit = await execute(client, {
    action: "commitSummary",
    project: "p",
    revision: "1",
  });
  assert.deepEqual(commit, { ...result, version: undefined, revision: 1n });
  assert.deepEqual(calls, [
    ["p", "w"],
    ["p", 1n],
  ]);
});

test("simultaneous summaries share requests, mutations and retries read fresh data", async () => {
  const load = sharedRequests<number>();
  let calls = 0;
  const fetch = async () => ++calls;
  const a = load("workspace-v1", fetch),
    b = load("workspace-v1", fetch);
  assert.equal(a, b);
  assert.deepEqual(await Promise.all([a, b]), [1, 1]);
  assert.equal(await load("workspace-v1", fetch), 2);
  assert.equal(await load("workspace-v2", fetch), 3);
  await assert.rejects(
    load("failed", async () => {
      throw new Error("retry");
    }),
  );
  assert.equal(await load("failed", fetch), 4);
});

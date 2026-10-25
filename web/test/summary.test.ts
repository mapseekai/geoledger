import assert from "node:assert/strict";
import test from "node:test";
import { execute } from "../src/lib/operations";
import { stubConsoleClient } from "./fixtures/console-client";

test("summary uses one aggregate call without loading names or feature bodies", async () => {
  const calls: unknown[] = [];
  const result = {
    total: { added: 9384n, deleted: 0n, modified: 0n },
    datasets: [
      { id: "d", name: "LUCC", added: 9384n, deleted: 0n, modified: 0n },
    ],
    schemaChanges: [
      {
        dataset: "d",
        before: [],
        after: [{ name: "note", type: "text", nullable: true, editable: true }],
      },
    ],
    version: 503n,
  };
  const client = stubConsoleClient({
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
  });
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

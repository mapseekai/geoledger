import assert from "node:assert/strict";
import test from "node:test";
import { sharedRequests } from "./change-summary";

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

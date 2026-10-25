import assert from "node:assert/strict";
import test from "node:test";
import {
  hasFeatureChanges,
  isEmptySummary,
  schemaFieldChanges,
  sharedRequests,
  type ChangeSummary,
} from "./change-summary";

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

test("schema-only drafts can publish while a known empty summary remains blocked", () => {
  const zero: ChangeSummary = {
    total: { added: "0", modified: 0, deleted: "0" },
    datasets: [],
  };
  const schemaChange = {
    dataset: "roads",
    before: [],
    after: [{ name: "note", type: "text", nullable: true, editable: true }],
  };
  assert.equal(isEmptySummary(undefined), false);
  assert.equal(isEmptySummary(zero), true);
  assert.equal(isEmptySummary({ ...zero, schemaChanges: [] }), true);
  assert.equal(hasFeatureChanges(zero.total), false);
  assert.equal(
    isEmptySummary({ ...zero, schemaChanges: [schemaChange] }),
    false,
  );
  for (const key of ["added", "modified", "deleted"] as const) {
    const total = { ...zero.total, [key]: "9007199254740993" };
    assert.equal(hasFeatureChanges(total), true);
    assert.equal(isEmptySummary({ ...zero, total }), false);
  }
});

test("field summaries show replacement and metadata changes even with unchanged field counts", () => {
  const field = (name: string, type = "text") => ({
    name,
    type,
    nullable: true,
    editable: true,
  });
  const change = {
    dataset: "roads",
    before: [field("old"), field("same"), field("typed")],
    after: [field("new"), field("same"), field("typed", "int8")],
  };
  assert.deepEqual(schemaFieldChanges(change), {
    added: ["new"],
    deleted: ["old"],
    modified: ["typed"],
  });
  assert.deepEqual(
    schemaFieldChanges({
      dataset: "roads",
      before: [field("id")],
      after: [{ ...field("id"), editable: false }],
    }),
    { added: [], deleted: [], modified: ["id"] },
  );
  assert.deepEqual(
    schemaFieldChanges({
      dataset: "roads",
      before: [field("id")],
      after: [{ ...field("id"), nullable: false }],
    }),
    { added: [], deleted: [], modified: ["id"] },
  );
  assert.deepEqual(
    schemaFieldChanges({
      dataset: "roads",
      before: [field("same")],
      after: [field("same")],
    }),
    { added: [], deleted: [], modified: [] },
  );
});

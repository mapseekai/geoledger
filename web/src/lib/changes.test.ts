import assert from "node:assert/strict";
import test from "node:test";
import { diffRows, featureName, parseChange } from "./changes";

const feature = (
  properties: Record<string, unknown>,
  geometry: unknown = null,
) => ({
  type: "Feature",
  properties,
  geometry,
});
test("changes classify workspace diffs and commit changes", () => {
  const added = parseChange(
    "c",
    JSON.stringify({
      dataset: "d",
      featureId: "f",
      base: null,
      draft: feature({ name: "锦里" }),
    }),
  );
  assert.equal(added.kind, "added");
  assert.equal(featureName(added.after), "锦里");
  const removed = parseChange(
    "c",
    JSON.stringify({
      dataset: "d",
      featureId: "f",
      before: feature({}),
      after: null,
    }),
  );
  assert.equal(removed.kind, "deleted");
  const modified = parseChange(
    "c",
    JSON.stringify({
      dataset: "d",
      featureId: "f",
      before: feature({ beds: 4300, x: 1 }),
      after: feature({ beds: 4500, x: 1 }),
    }),
  );
  const rows = diffRows(modified);
  assert.equal(rows.find((r) => r.key === "beds")!.changed, true);
  assert.equal(rows.find((r) => r.key === "x")!.changed, false);
  assert.equal(rows[0].changed, false);
});

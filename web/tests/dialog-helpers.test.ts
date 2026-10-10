import assert from "node:assert/strict";
import { test } from "node:test";
import { diffRows, featureName, parseChange } from "../src/lib/changes";
import {
  attributeRows,
  choiceKind,
  conflictKind,
  parseConflict,
  resolutionOf,
  resultSlot,
  sideSelection,
} from "../src/lib/conflicts";
import { count, geometryLabel, relativeTime } from "../src/lib/format";
import { previewShapes } from "../src/lib/geometry-preview";

const feature = (
  properties: Record<string, unknown>,
  geometry: unknown = null,
) => ({
  type: "Feature",
  properties,
  geometry,
});
const fieldConflict = () =>
  parseConflict(
    JSON.stringify({
      dataset: "roads",
      featureId: "r1",
      fields: ["/properties/lanes"],
      base: feature({ name: "人民南路", lanes: 6, speed: 60 }),
      current: feature({ name: "人民南路", lanes: 8, speed: 60 }),
      draft: feature({ name: "人民南路", lanes: 10, speed: 50 }),
    }),
  );

test("side selection keeps one side for conflicting fields and merges the rest", () => {
  const conflict = fieldConflict();
  const mine = resolutionOf(conflict, sideSelection(conflict, "draft"))!;
  assert.match(mine, /"lanes":10/);
  assert.match(mine, /"speed":50/);
  const theirs = resolutionOf(conflict, sideSelection(conflict, "current"))!;
  assert.match(theirs, /"lanes":8/);
  // Non-conflicting draft edits survive "take theirs" for the conflicting field.
  assert.match(theirs, /"speed":50/);
  assert.equal(choiceKind(conflict, sideSelection(conflict, "draft")), "draft");
  assert.equal(choiceKind(conflict, undefined), undefined);
  assert.equal(choiceKind(conflict, { fields: {}, text: mine }), "custom");
});

test("whole-feature conflicts resolve to deletion or the chosen feature", () => {
  const conflict = parseConflict(
    JSON.stringify({
      dataset: "poi",
      featureId: "p1",
      fields: ["*"],
      base: feature(
        { name: "杜甫草堂" },
        { type: "Point", coordinates: [104.028, 30.66] },
      ),
      current: feature(
        { name: "杜甫草堂博物馆" },
        { type: "Point", coordinates: [104.0283, 30.6602] },
      ),
      draft: null,
    }),
  );
  assert.equal(conflictKind(conflict), "deleted-draft");
  assert.equal(resolutionOf(conflict, sideSelection(conflict, "draft")), null);
  assert.match(
    resolutionOf(conflict, sideSelection(conflict, "current"))!,
    /杜甫草堂博物馆/,
  );
  const rows = attributeRows(conflict);
  assert.equal(rows[0].label, "几何");
  assert.ok(rows.every((row) => row.conflict));
  assert.equal(resultSlot(null, rows[1]).present, false);
});

test("attribute rows flag only server-reported conflicts and keep exact numbers", () => {
  const conflict = parseConflict(
    '{"dataset":"d","featureId":"f","fields":["/properties/a"],"base":{"type":"Feature","properties":{"a":1,"big":18446744073709551615},"geometry":null},"current":{"type":"Feature","properties":{"a":2,"big":18446744073709551615},"geometry":null},"draft":{"type":"Feature","properties":{"a":3,"big":18446744073709551615,"b":true},"geometry":null}}',
  );
  const rows = attributeRows(conflict);
  const byKey = Object.fromEntries(rows.map((row) => [row.label, row]));
  assert.equal(byKey.a.conflict, true);
  assert.equal(byKey.b.conflict, false);
  assert.equal(byKey.b.changed, true);
  assert.equal(byKey.big.changed, false);
  assert.equal(conflictKind(conflict), "attributes");
  const text = resolutionOf(conflict, sideSelection(conflict, "draft"))!;
  assert.match(text, /18446744073709551615/);
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

test("formatting helpers", () => {
  assert.equal(count("1234567"), "1,234,567");
  assert.equal(count(undefined), "–");
  assert.equal(
    geometryLabel({
      type: "LineString",
      coordinates: [
        [0, 0],
        [1, 1],
        [2, 2],
      ],
    }),
    "LineString · 3 点",
  );
  assert.equal(
    geometryLabel({
      type: "Polygon",
      coordinates: [
        [
          [0, 0],
          [1, 0],
          [1, 1],
          [0, 0],
        ],
      ],
    }),
    "Polygon · 4 点",
  );
  assert.equal(geometryLabel(null), "无几何");
  const now = Date.parse("2026-10-10T12:00:00Z");
  assert.equal(relativeTime("2026-10-10T11:59:50Z", now), "刚刚");
  assert.equal(relativeTime("2026-10-10T11:55:00Z", now), "5分钟前");
  assert.equal(relativeTime("2026-10-10T09:00:00Z", now), "3小时前");
});

test("geometry preview projects all versions into one viewport", () => {
  const [a, b] = previewShapes(
    [
      { type: "Point", coordinates: [104, 30] },
      {
        type: "LineString",
        coordinates: [
          [104, 30],
          [104.01, 30.01],
        ],
      },
    ],
    { width: 100, height: 100, padding: 10 },
  );
  assert.equal(a[0].kind, "point");
  assert.equal(b[0].kind, "path");
  const [empty] = previewShapes([null], {
    width: 100,
    height: 100,
    padding: 10,
  });
  assert.deepEqual(empty, []);
});

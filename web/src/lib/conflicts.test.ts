import assert from "node:assert/strict";
import { stringify } from "lossless-json";
import test from "node:test";
import {
  mergeConflict,
  parseConflict,
  pointerForProperty,
  attributeRows,
  choiceKind,
  conflictKind,
  resolutionOf,
  resultSlot,
  sideSelection,
} from "./conflicts";

const feature = (
  properties: Record<string, unknown>,
  geometry: unknown = null,
) => ({
  type: "Feature",
  properties,
  geometry,
});

test("field merge preserves lossless numeric tokens while combining independent edits", () => {
  const conflict = parseConflict(
    '{"dataset":"parcels","featureId":"a","fields":["/properties/name"],"base":{"type":"Feature","properties":{"name":"old","exact":18446744073709551615},"geometry":null},"current":{"type":"Feature","properties":{"name":"published","exact":18446744073709551615},"geometry":null},"draft":{"type":"Feature","properties":{"name":"draft","exact":18446744073709551615,"note":"mine"},"geometry":null}}',
  );
  const result = mergeConflict(conflict, { "/properties/name": "draft" });
  assert.match(result!.text, /18446744073709551615/);
  assert.match(result!.text, /"note":"mine"/);
  assert.match(result!.text, /"name":"draft"/);
});

test("field merge distinguishes a missing property from a null property", () => {
  const conflict = parseConflict(
    JSON.stringify({
      dataset: "parcels",
      featureId: "a",
      fields: ["/properties/value"],
      base: feature({ value: "old" }),
      current: feature({}),
      draft: feature({ value: null }),
    }),
  );
  assert.equal(
    mergeConflict(conflict, { "/properties/value": "current" })!.feature
      .properties!.value,
    undefined,
  );
  assert.equal(
    mergeConflict(conflict, { "/properties/value": "draft" })!.feature
      .properties!.value,
    null,
  );
});

test("property pointers escape slash and tilde without conflating keys", () => {
  assert.equal(pointerForProperty("a/b~c"), "/properties/a~1b~0c");
  const conflict = parseConflict(
    JSON.stringify({
      dataset: "parcels",
      featureId: "a",
      fields: ["/properties/a~1b~0c"],
      base: feature({ "a/b~c": 1, plain: 1 }),
      current: feature({ "a/b~c": 2, plain: 1 }),
      draft: feature({ "a/b~c": 3, plain: 2 }),
    }),
  );
  const result = mergeConflict(conflict, { "/properties/a~1b~0c": "current" });
  assert.equal(stringify(result!.feature.properties!["a/b~c"]), "2");
  assert.equal(stringify(result!.feature.properties!.plain), "2");
});

test("stale resolutions require explicit whole-feature confirmation", () => {
  const conflict = parseConflict(
    JSON.stringify({
      dataset: "parcels",
      featureId: "a",
      fields: [],
      reason: "stale_resolution",
      base: feature({ value: 1 }),
      current: feature({ value: 2 }),
      draft: feature({ value: 3 }),
    }),
  );
  assert.equal(mergeConflict(conflict, {}), undefined);
  assert.equal(
    stringify(
      mergeConflict(conflict, { "*": "draft" })!.feature.properties!.value,
    ),
    "3",
  );
});
test("conflict parsing preserves an own __proto__ property", () => {
  const conflict = parseConflict(
    '{"dataset":"parcels","featureId":"a","fields":["/properties/__proto__"],"base":{"type":"Feature","properties":{"__pr\\u006fto__":1},"geometry":null},"current":{"type":"Feature","properties":{"__pr\\u006fto__":2},"geometry":null},"draft":{"type":"Feature","properties":{"__pr\\u006fto__":3},"geometry":null}}',
  );
  const result = mergeConflict(conflict, { "/properties/__proto__": "draft" })!;
  assert(Object.hasOwn(result.feature.properties!, "__proto__"));
  assert.equal(stringify(result.feature.properties!["__proto__"]), "3");
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

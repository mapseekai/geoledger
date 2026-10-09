import assert from "node:assert/strict";
import { stringify } from "lossless-json";
import { test } from "node:test";
import {
  mergeConflict,
  parseConflict,
  pointerForProperty,
} from "../src/lib/conflicts";

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

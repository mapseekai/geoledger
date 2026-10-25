"use strict";
const test = require("node:test");
const assert = require("node:assert/strict");

test("transport decoders reject invalid Feature envelopes and dimensions without rounding values", () => {
  const convert = require("../../dist/_internal/convert");
  const dataset = { dataset: "d", name: "D", geometryType: "point", coordinateDimension: 2, crs: "EPSG:4326" };
  for (const coordinateDimension of [0, 9, "2", undefined])
    assert.throws(() => convert.decodeDataset({ ...dataset, coordinateDimension }), /coordinate dimension/);
  for (const coordinateDimension of [2, 3])
    assert.equal(convert.decodeDataset({ ...dataset, coordinateDimension }).coordinateDimension, coordinateDimension);
  const page = (geojson) => ({ features: [{ geojson }], revision: "9007199254740993", crs: "EPSG:4326", nextAfter: "opaque/%/cursor" });
  for (const raw of ["null", "[]", "{}", '{"type":"Feature","id":1,"properties":{},"geometry":null}', '{"type":"Feature","id":"f","properties":[],"geometry":null}', '{"type":"Feature","id":"f","properties":{}}'])
    assert.throws(() => convert.decodeFeaturePage(page(raw)), /Feature/);
  const result = convert.decodeFeaturePage(page('{"type":"Feature","id":"f","properties":{"exact":18446744073709551615,"__proto__":{"safe":1},"snake_case":7},"geometry":null}'));
  assert.equal(result.revision, 9007199254740993n);
  assert.equal(result.nextAfter, "opaque/%/cursor");
  assert.equal(result.features[0].properties.exact, 18446744073709551615n);
  assert.equal(Object.hasOwn(result.features[0].properties, "__proto__"), true);
  assert.equal(result.features[0].properties.snake_case, 7);
});


test("query page decoders preserve opaque and 64-bit continuations and terminal absence", () => {
  const convert = require("../../dist/_internal/convert");
  for (const nextAfter of [undefined, "opaque/%/cursor"]) {
    for (const [decode, value] of [
      [convert.decodeDiff, { baseRevision: "1", version: "2", changes: [], schemaChanges: [], nextAfter }],
      [convert.decodeCommitChanges, { revision: "3", changes: [], schemaChanges: [], nextAfter }],
    ]) {
      const result = decode(value);
      assert.equal(result.nextAfter, nextAfter);
      assert.equal(Object.hasOwn(result, "nextAfter"), nextAfter !== undefined);
    }
  }
  for (const nextAfter of [undefined, "9007199254740993"]) {
    for (const [decode, value] of [
      [convert.decodeHistoryPage, { commits: [], nextAfter }],
      [convert.decodeAuditPage, { events: [], nextAfter }],
    ]) {
      const result = decode(value);
      assert.equal(result.nextAfter, nextAfter === undefined ? undefined : 9007199254740993n);
      assert.equal(Object.hasOwn(result, "nextAfter"), nextAfter !== undefined);
    }
  }
});

test("schema-only changes remain visible in draft, commit and summary models", () => {
  const convert=require("../../dist/_internal/convert");
  const change={dataset:"d",before:[],after:[{name:"note",type:"text",nullable:true,editable:true}]};
  const diff=convert.decodeDiff({baseRevision:"1",version:"2",changes:[],schemaChanges:[change]});
  const commit=convert.decodeCommitChanges({revision:"2",changes:[],schemaChanges:[change]});
  const summary=convert.decodeChangeSummary({total:{added:"0",deleted:"0",modified:"0"},datasets:[],schemaChanges:[change]});
  for(const result of [diff,commit,summary]) assert.deepEqual(result.schemaChanges,[change]);
});

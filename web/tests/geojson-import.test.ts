import assert from "node:assert/strict";
import test from "node:test";
import { importBatch, importGeojson } from "../src/lib/geojson-import";

test("imports collections preserving numeric IDs and exact property tokens", () => {
  const rows = importGeojson(
    '{"type":"FeatureCollection","features":[{"type":"Feature","id":18446744073709551615,"geometry":null,"properties":{"n":1.00000000000000001e18}},{"type":"Feature","geometry":null,"properties":null}]}',
    () => "generated",
  );
  assert.equal(rows[0].id, "18446744073709551615");
  assert.match(rows[0].raw, /1.00000000000000001e18/);
  assert.match(rows[0].raw, /"id":"18446744073709551615"/);
  assert.equal(rows[1].id, "generated");
});
test("accepts BOM and standalone geometry", () => {
  const [row] = importGeojson(
    '\uFEFF{"type":"Point","coordinates":[104,35]}',
    () => "point",
  );
  assert.deepEqual(JSON.parse(row.raw), {
    type: "Feature",
    properties: {},
    geometry: { type: "Point", coordinates: [104, 35] },
    id: "point",
  });
});
test("rejects invalid, empty and duplicate input before saving", () => {
  const feature = {
    type: "Feature",
    id: "same",
    properties: {},
    geometry: null,
  };
  for (const raw of [
    "bad",
    "null",
    '{"type":"FeatureCollection","features":[]}',
    JSON.stringify({ ...feature, properties: [] }),
    JSON.stringify({ type: "FeatureCollection", features: [feature, feature] }),
  ]) {
    assert.throws(() => importGeojson(raw));
  }
});

test("adapts null properties and bounding boxes to storage", () => {
  const [row] = importGeojson(
    JSON.stringify({
      type: "Feature",
      id: "point",
      properties: null,
      bbox: [1, 2, 1, 2],
      geometry: { type: "Point", coordinates: [1, 2], bbox: [1, 2, 1, 2] },
    }),
  );
  assert.deepEqual(JSON.parse(row.raw), {
    type: "Feature",
    id: "point",
    properties: {},
    geometry: { type: "Point", coordinates: [1, 2] },
  });
});

test("accepts files larger than 1 MiB and more than 100 features", () => {
  const features = Array.from({ length: 1100 }, (_, i) => ({
    type: "Feature",
    id: String(i),
    properties: { text: "a".repeat(1024) },
    geometry: null,
  }));
  const raw = JSON.stringify({ type: "FeatureCollection", features });
  assert.ok(raw.length > 1024 * 1024);
  assert.equal(importGeojson(raw).length, 1100);
});

test("large individual features have no byte cap and batches adapt to payload size", () => {
  const features = Array.from({ length: 4 }, (_, i) => ({
    type: "Feature",
    id: String(i),
    properties: { text: "a".repeat(300 * 1024) },
    geometry: null,
  }));
  const rows = importGeojson(
    JSON.stringify({ type: "FeatureCollection", features }),
  );
  assert.equal(rows.length, 4);
  assert.equal(importBatch(rows, 0).length, 1);
  assert.equal(importBatch(rows, 1)[0].id, "1");
});

test("reprojects declared Web Mercator and preserves properties and altitude", () => {
  const [row] = importGeojson(
    '{"type":"Feature","crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::3857"}},"properties":{"exact":9007199254740993},"geometry":{"type":"Point","coordinates":[1113194.9079327357,1118889.9748579594,12]}}',
    () => "p",
    "point",
  );
  assert.equal(row.reprojected, true);
  assert.match(row.raw, /9007199254740993/);
  const coords = JSON.parse(row.raw).geometry.coordinates;
  assert.ok(
    Math.abs(coords[0] - 10) < 1e-10 && Math.abs(coords[1] - 10) < 1e-10,
  );
  assert.equal(coords[2], 12);
});
test("rejects unknown CRS, undeclared projected coordinates and mixed families", () => {
  assert.throws(
    () =>
      importGeojson(
        '{"type":"Point","crs":{"type":"name","properties":{"name":"EPSG:9999"}},"coordinates":[1,2]}',
      ),
    /坐标系/,
  );
  assert.throws(
    () => importGeojson('{"type":"Point","coordinates":[1000000,2000000]}'),
    /经纬度/,
  );
  assert.throws(
    () =>
      importGeojson(
        '{"type":"MultiPoint","coordinates":[[1,2]]}',
        undefined,
        "polygon",
      ),
    /面类型/,
  );
  assert.equal(
    importGeojson(
      '{"type":"MultiPolygon","coordinates":[[[[0,0],[1,0],[1,1],[0,0]]]]}',
      undefined,
      "polygon",
    ).length,
    1,
  );
});

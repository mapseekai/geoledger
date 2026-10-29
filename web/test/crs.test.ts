import assert from "node:assert/strict";
import test from "node:test";
import { datasetCrs } from "../src/lib/crs-server";
import { buildLayer } from "../src/lib/map";

test("EPSG directory and custom definitions preview native data without modifying it", () => {
  for (const [crs, coordinates, expected] of [
    ["EPSG:32650", [500000, 0], [117, 0]],
    ["EPSG:4490", [117, 30], [117, 30]],
    [
      "+proj=utm +zone=50 +datum=WGS84 +units=m +no_defs",
      [500000, 0],
      [117, 0],
    ],
  ] as const) {
    const dataset = datasetCrs({
      id: "d",
      name: "native",
      geometryType: "point",
      coordinateDimension: 2,
      crs,
    });
    const raw = JSON.stringify({
      type: "Feature",
      id: "p",
      properties: {},
      geometry: { type: "Point", coordinates },
    });
    const layer = buildLayer(
      [{ id: "p", geojson: raw }],
      dataset.crsDefinition,
    );
    assert.equal(layer.items[0].raw, raw);
    assert.equal(layer.counts.point, 1);
    assert.ok(Math.abs(layer.bounds![0] - expected[0]) < 1e-7);
    assert.ok(Math.abs(layer.bounds![1] - expected[1]) < 1e-7);
  }
});

import { test } from "node:test";
import assert from "node:assert/strict";
import { rasterBasemap } from "../src/lib/basemap";
import {
  buildLayer,
  contains,
  formatLngLat,
  layerItem,
  matchesQuery,
  mercatorBounds,
  pageContaining,
  pageOf,
  propertyRows,
  wrapLongitude,
} from "../src/lib/map";

const feature = (geometry: unknown, properties: unknown = {}) =>
  JSON.stringify({ type: "Feature", properties, geometry });

test("layer items classify geometry kinds and count parts and vertices", () => {
  const point = layerItem(
    "p",
    feature({ type: "Point", coordinates: [104, 30] }),
    0,
  );
  assert.equal(point.kind, "point");
  assert.deepEqual(point.bounds, [104, 30, 104, 30]);
  const multi = layerItem(
    "m",
    feature({
      type: "MultiPolygon",
      coordinates: [
        [
          [
            [0, 0],
            [1, 0],
            [1, 1],
            [0, 0],
          ],
        ],
        [
          [
            [2, 2],
            [3, 2],
            [3, 3],
            [2, 2],
          ],
        ],
      ],
    }),
    1,
  );
  assert.equal(multi.kind, "polygon");
  assert.equal(multi.parts, 2);
  assert.equal(multi.vertices, 8);
  assert.deepEqual(multi.bounds, [0, 0, 3, 3]);
  const mixed = layerItem(
    "c",
    feature({
      type: "GeometryCollection",
      geometries: [
        { type: "Point", coordinates: [5, 5] },
        {
          type: "LineString",
          coordinates: [
            [0, 0],
            [1, 1],
          ],
        },
      ],
    }),
    2,
  );
  assert.equal(mixed.kind, "mixed");
  assert.deepEqual(mixed.draws, ["point", "line"]);
  const single = layerItem(
    "s",
    feature({
      type: "GeometryCollection",
      geometries: [
        {
          type: "LineString",
          coordinates: [
            [0, 0],
            [1, 1],
          ],
        },
      ],
    }),
    3,
  );
  assert.equal(single.kind, "line");
});

test("layer items never throw on empty, invalid or unparsable geometry", () => {
  assert.equal(layerItem("n", feature(null), 0).kind, "none");
  assert.equal(
    layerItem("e", feature({ type: "MultiPoint", coordinates: [] }), 0).kind,
    "none",
  );
  const invalid = layerItem(
    "i",
    feature({ type: "Point", coordinates: [0, 91] }),
    0,
  );
  assert.equal(invalid.kind, "invalid");
  assert.ok(invalid.error);
  assert.equal(
    layerItem("t", feature({ type: "Circle", coordinates: [0, 0] }), 0).kind,
    "invalid",
  );
  assert.equal(layerItem("x", "{not json", 0).kind, "invalid");
});

test("properties keep exact numbers and labels come from name fields", () => {
  const raw =
    '{"type":"Feature","properties":{"name":"人民南路","big":18446744073709551615,"ok":true,"none":null,"tags":["a",1]},"geometry":null}';
  const rows = propertyRows(raw);
  assert.deepEqual(
    rows.map((r) => [r.key, r.value, r.kind]),
    [
      ["name", "人民南路", "string"],
      ["big", "18446744073709551615", "number"],
      ["ok", "true", "boolean"],
      ["none", "null", "null"],
      ["tags", '["a",1]', "object"],
    ],
  );
  assert.equal(layerItem("a", raw, 0).label, "人民南路");
  assert.deepEqual(propertyRows('{"type":"Feature","properties":[1]}'), []);
});

test("layer model unions bounds, counts kinds and keeps drawable features only", () => {
  const layer = buildLayer([
    {
      id: "a",
      geojson: feature(
        { type: "Point", coordinates: [104, 30] },
        { name: "A", x: 1 },
      ),
    },
    {
      id: "b",
      geojson: feature(
        {
          type: "LineString",
          coordinates: [
            [103, 31],
            [105, 29],
          ],
        },
        { y: 2 },
      ),
    },
    { id: "c", geojson: feature(null, { x: 3 }) },
  ]);
  assert.deepEqual(layer.bounds, [103, 29, 105, 31]);
  assert.equal(layer.counts.point, 1);
  assert.equal(layer.counts.line, 1);
  assert.equal(layer.counts.none, 1);
  assert.deepEqual(layer.columns, ["name", "x", "y"]);
  assert.deepEqual(
    layer.collection.features.map((f) => f.id),
    [0, 1],
  );
  assert.deepEqual(layer.collection.features[0].properties, {});
  assert.equal(buildLayer([]).bounds, null);
});

test("search matches ids, property names and values case-insensitively", () => {
  const item = layerItem(
    "Road-001",
    feature(null, { name: "人民南路", class: "Primary" }),
    0,
  );
  assert.ok(matchesQuery(item, "road"));
  assert.ok(matchesQuery(item, "人民"));
  assert.ok(matchesQuery(item, "primary"));
  assert.ok(matchesQuery(item, "CLASS"));
  assert.ok(matchesQuery(item, "  "));
  assert.ok(!matchesQuery(item, "river"));
});

test("paging clamps pages and locates rows", () => {
  const rows = Array.from({ length: 250 }, (_, i) => i);
  assert.deepEqual(pageOf(rows, 0, 100).rows.length, 100);
  assert.equal(pageOf(rows, 9, 100).page, 2);
  assert.equal(pageOf(rows, 9, 100).rows.length, 50);
  assert.equal(pageOf([], 3, 100).pages, 1);
  assert.equal(
    pageContaining(rows, (r) => r === 199, 100),
    1,
  );
  assert.equal(
    pageContaining(rows, (r) => r === 999, 100),
    undefined,
  );
});

test("coordinates are wrapped, formatted and clamped for Web Mercator", () => {
  assert.equal(wrapLongitude(190), -170);
  assert.equal(wrapLongitude(-190), 170);
  assert.equal(wrapLongitude(180), 180);
  assert.equal(formatLngLat(104.065712, 30.657311), "104.06571, 30.65731");
  assert.deepEqual(
    mercatorBounds([0, -90, 1, 90]),
    [0, -85.051129, 1, 85.051129],
  );
  assert.ok(contains([0, 0, 10, 10], [1, 1, 2, 2]));
  assert.ok(!contains([0, 0, 10, 10], [1, 1, 12, 2]));
});

test("raster basemap is opt-in and only accepts HTTP(S) XYZ templates", () => {
  assert.equal(rasterBasemap({}), undefined);
  assert.deepEqual(
    rasterBasemap({
      GL_WEB_BASEMAP_URL: "https://tile.openstreetmap.org/{z}/{x}/{y}.png",
      GL_WEB_BASEMAP_ATTRIBUTION: "© OpenStreetMap contributors",
    }),
    {
      url: "https://tile.openstreetmap.org/{z}/{x}/{y}.png",
      origin: "https://tile.openstreetmap.org",
      attribution: "© OpenStreetMap contributors",
    },
  );
  assert.equal(
    rasterBasemap({
      GL_WEB_BASEMAP_URL: "http://10.0.0.5:8080/{z}/{x}/{y}.png",
    })?.attribution,
    "10.0.0.5",
  );
  for (const url of [
    "https://tiles.example/{z}/{x}.png",
    "ftp://tiles.example/{z}/{x}/{y}.png",
    "javascript:alert(1)//{z}{x}{y}",
    "https://tiles.example/{z}/{x}/{y}.png; script-src *",
    "https://user:pw@tiles.example/{z}/{x}/{y}.png",
  ])
    assert.equal(rasterBasemap({ GL_WEB_BASEMAP_URL: url }), undefined, url);
});

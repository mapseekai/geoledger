import assert from "node:assert/strict";
import test from "node:test";
import { featureText, pretty, geometryType, previewGeometry } from "./geojson";

test("feature editing and display preserve uint64, exponent tokens and user field names", () => {
  const input =
    '{"type":"Feature","properties":{"exact":18446744073709551615,"geojson":"keep","detail_json":"keep","exponent":1.00000000000000001e18},"geometry":null}';
  const edited = featureText(pretty(input), "a");
  assert.match(edited, /18446744073709551615/);
  assert.match(edited, /1.00000000000000001e18/);
  assert.match(edited, /"id":"a"/);
  assert.throws(() => featureText('{"type":"Feature","id":"other"}', "a"));
  assert.throws(() => featureText("null", "a"));
});

test("geometry type labels tolerate missing or invalid geometry", () => {
  assert.equal(
    geometryType(
      '{"type":"Feature","properties":{"n":18446744073709551615},"geometry":{"type":"LineString","coordinates":[[0,0],[1,1]]}}',
    ),
    "LineString",
  );
  assert.equal(geometryType('{"type":"Feature","geometry":null}'), "无几何");
  assert.equal(geometryType("not json"), "未知");
});

test("map preview bounds cover collections and preserve the original exact text", () => {
  const raw =
    '{"type":"Feature","properties":{"exact":18446744073709551615},"geometry":{"type":"GeometryCollection","geometries":[{"type":"Point","coordinates":[104,35]},{"type":"MultiLineString","coordinates":[[[100,30],[110,40]]]},{"type":"Polygon","coordinates":[[[99,29],[111,29],[99,41],[99,29]]]}]}}';
  const preview = previewGeometry(raw);
  assert.deepEqual(preview.bounds, [99, 29, 111, 41]);
  assert.deepEqual(preview.data.properties, {});
  assert.match(pretty(raw), /18446744073709551615/);
  for (const geometry of [
    null,
    { type: "GeometryCollection", geometries: [] },
    { type: "MultiPoint", coordinates: [] },
  ])
    assert.equal(
      previewGeometry(JSON.stringify({ type: "Feature", geometry })).bounds,
      null,
    );
  assert.throws(() =>
    previewGeometry(
      '{"type":"Feature","geometry":{"type":"Point","coordinates":[0,91]}}',
    ),
  );
  assert.throws(() =>
    previewGeometry(
      '{"type":"Feature","geometry":{"type":"Point","coordinates":[0,null]}}',
    ),
  );
});

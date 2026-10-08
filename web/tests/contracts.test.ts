import { test } from "node:test";
import assert from "node:assert/strict";
import { Client } from "@geoledger/client";
import { execute, encode } from "../src/lib/operations";
import {
  featureText,
  geometryType,
  pretty,
  previewGeometry,
} from "../src/lib/geojson";
import {
  publication,
  readPublication,
  releasePublication,
} from "../src/lib/publication";
import { requestSchema } from "../src/lib/requests";
import { configuration } from "../src/lib/config";

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
test("BFF uses business SDK with exact revisions and immutable publication", async () => {
  const intent = publication(
    "project",
    "workspace",
    "9007199254740993",
    "message",
  );
  const restored = readPublication(JSON.stringify(intent))!;
  assert.deepEqual(restored, intent);
  assert(Object.isFrozen(restored));
  const calls: unknown[] = [];
  const client = {
    publish: async (v: unknown) => {
      calls.push(v);
      return { revision: 9007199254740994n };
    },
  } as unknown as Client;
  assert.equal(
    encode(await execute(client, restored)),
    '{"revision":"9007199254740994"}',
  );
  await execute(client, restored);
  assert.deepEqual(calls[0], calls[1]);
  assert.equal(
    (calls[0] as { expectedWorkspaceVersion: bigint }).expectedWorkspaceVersion,
    9007199254740993n,
  );
});
test("BFF transports feature JSON as text and never converts property integers to strings", async () => {
  const client = {
    features: async () => ({
      revision: 5n,
      features: [
        {
          id: "a",
          type: "Feature",
          properties: { big: 18446744073709551615n, geojson: "untouched" },
          geometry: null,
        },
      ],
    }),
  } as unknown as Client;
  const result = (await execute(client, {
    action: "features",
    project: "p",
    dataset: "d",
  })) as { features: { geojson: string }[] };
  assert.match(result.features[0].geojson, /"big":18446744073709551615/);
  assert.match(result.features[0].geojson, /"geojson":"untouched"/);
});
test("invalid actions, extra upstream addresses, missing delete values and unsafe revisions fail closed", () => {
  for (const value of [
    { action: "unknown" },
    { action: "info", endpoint: "http://attacker" },
    {
      action: "save",
      project: "p",
      workspace: "w",
      version: "1",
      edits: [{ dataset: "d", featureId: "f" }],
    },
    {
      action: "publish",
      project: "p",
      workspace: "w",
      version: 9007199254740992,
      requestId: crypto.randomUUID(),
      message: "m",
    },
    { action: "history", project: "p", after: "9223372036854775808" },
    { action: "setMember", project: "p", subject: "s", role: "none" },
  ])
    assert.equal(requestSchema.safeParse(value).success, false);
  assert(
    requestSchema.safeParse({
      action: "save",
      project: "p",
      workspace: "w",
      version: "1",
      edits: [{ dataset: "d", featureId: "f", feature: null }],
    }).success,
  );
});
test("deployments require a secret and HTTPS for non-loopback production origins", () => {
  const saved = { ...process.env };
  try {
    delete process.env.GL_WEB_SESSION_SECRET;
    assert.throws(configuration, /SECRET/);
    process.env.GL_WEB_SESSION_SECRET = "test-only-secret-".repeat(3);
    Object.assign(process.env, { NODE_ENV: "production" });
    process.env.GL_WEB_ORIGIN = "http://console.example.com";
    assert.throws(configuration, /HTTPS/);
    process.env.GL_WEB_ORIGIN = "https://console.example.com";
    assert.equal(configuration().secure, true);
    process.env.GL_WEB_ORIGIN = "http://127.0.0.1:3000";
    assert.equal(configuration().secure, false);
  } finally {
    for (const name of ["GL_WEB_SESSION_SECRET", "NODE_ENV", "GL_WEB_ORIGIN"]) {
      if (saved[name] === undefined) delete process.env[name];
      else process.env[name] = saved[name];
    }
  }
});

test("pending publication survives expired authentication but releases definitive merge conflicts", () => {
  for (const status of [401, 403, 404])
    assert.equal(releasePublication(status, false, true), false);
  assert.equal(releasePublication(409, false, true), true);
  assert.equal(releasePublication(400, false, true), true);
  assert.equal(releasePublication(503, true, true), false);
  assert.equal(releasePublication(401, false, false), true);
});
test("existing 256-byte feature identifiers remain editable through the web contract", () => {
  const edit = { dataset: "d", featureId: "a".repeat(256), feature: null };
  assert(
    requestSchema.safeParse({
      action: "save",
      project: "p",
      workspace: "w",
      version: "1",
      edits: [edit],
    }).success,
  );
  edit.featureId += "b";
  assert.equal(
    requestSchema.safeParse({
      action: "save",
      project: "p",
      workspace: "w",
      version: "1",
      edits: [edit],
    }).success,
    false,
  );
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

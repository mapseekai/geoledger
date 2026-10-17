import assert from "node:assert/strict";
import test from "node:test";
import { stringifyJson } from "@geoledger/client";
import { stubConsoleClient } from "./fixtures/console-client";
import { execute, encode } from "../src/lib/operations";
import { publication, readPublication } from "../src/lib/publication";
import { requestSchema } from "../src/lib/requests";

test("table tracking uses the private business transport and validates the source", async () => {
  const source = {
    schema: "business",
    table: "roads",
    idColumn: "id",
    geometryColumn: "geom",
  };
  const request = { action: "trackTable", project: "p", name: "roads", source };
  const client = stubConsoleClient({
    trackTable: async (...args: unknown[]) => {
      assert.deepEqual(args, ["p", "roads", source]);
      return {
        id: "d",
        name: "roads",
        geometryType: "line",
        coordinateDimension: 2,
        postgisTable: source,
        crs: "LOCAL:test",
      };
    },
  });
  assert.deepEqual(await execute(client, request), {
    id: "d",
    name: "roads",
    geometryType: "line",
    coordinateDimension: 2,
    postgisTable: source,
    crs: "LOCAL:test",
    crsDefinition: "LOCAL:test",
  });
  assert.equal(
    requestSchema.safeParse({ ...request, source: { ...source, table: "" } })
      .success,
    false,
  );
});

test("BFF preserves exact revisions and immutable publication", async () => {
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
  const client = stubConsoleClient({
    publish: async (v: unknown) => {
      calls.push(v);
      return {
        revision: 9007199254740994n,
        workspace: "workspace",
        version: 0n,
        status: "published",
        changes: 0n,
      };
    },
  });
  assert.equal(
    encode(await execute(client, restored)),
    '{"revision":"9007199254740994","workspace":"workspace","version":"0","status":"published","changes":"0"}',
  );
  await execute(client, restored);
  assert.deepEqual(calls[0], calls[1]);
  assert.equal(
    (calls[0] as { expectedWorkspaceVersion: bigint }).expectedWorkspaceVersion,
    9007199254740993n,
  );
});

test("BFF returns FeatureCollection and preserves numeric GeoJSON properties", async () => {
  const client = stubConsoleClient({
    features: async () => ({
      crs: "EPSG:4326",
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
  });
  const result = (await execute(client, {
    action: "features",
    project: "p",
    dataset: "d",
  })) as { type: string; features: unknown[] };
  assert.equal(result.type, "FeatureCollection");
  assert.match(stringifyJson(result.features[0]), /"big":18446744073709551615/);
  assert.match(stringifyJson(result.features[0]), /"geojson":"untouched"/);
});

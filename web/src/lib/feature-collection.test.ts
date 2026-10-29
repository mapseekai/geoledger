import type { ConsoleClient } from "./console-client";
import assert from "node:assert/strict";
import test from "node:test";
import { type Feature, stringifyJson } from "@geoledger/client";
import { featureCollection, featureQuery } from "./feature-collection";

test("GeoJSON next link preserves snapshot, bbox and limit without a geometry wrapper", async () => {
  const calls: unknown[] = [];
  const feature = {
    type: "Feature",
    id: "a/b",
    properties: { exact: 18446744073709551615n },
    geometry: null,
  } satisfies Feature;
  const client = {
    features: async (...args: unknown[]) => {
      calls.push(args);
      return {
        crs: "EPSG:3857",
        features: [feature],
        revision: 9n,
        nextAfter: "a/b",
      };
    },
  } satisfies Pick<ConsoleClient, "features">;
  const result = await featureCollection(client, {
    action: "features",
    project: "p",
    dataset: "d",
    limit: 1,
    bbox: [1, 2, 3, 4],
  });
  assert.equal(result.type, "FeatureCollection");
  assert.equal(result.crs.properties.name, "EPSG:3857");
  assert.equal(result.links[0].type, "application/json");
  assert.equal(result.numberReturned, 1);
  assert.equal(result.features[0], feature);
  assert.match(stringifyJson(result), /"exact":18446744073709551615/);
  const next = new URL(
    result.links.find((l) => l.rel === "next")!.href,
    "http://localhost",
  );
  assert.equal(next.pathname, "/api/projects/p/collections/d/items");
  assert.equal(next.searchParams.get("revision"), "9");
  assert.equal(next.searchParams.get("bbox"), "1,2,3,4");
  assert.equal(next.searchParams.get("after"), "a/b");
  assert.equal(next.searchParams.get("limit"), "1");
  await featureCollection(client, featureQuery("p", "d", next.searchParams));
  assert.equal(
    (calls[1] as unknown[])[2] &&
      ((calls[1] as unknown[])[2] as { revision: bigint }).revision,
    9n,
  );
});

test("workspace pagination detects changed draft and empty collections omit next", async () => {
  const client = {
    features: async () => ({
      crs: "EPSG:4326",
      features: [],
      revision: 0n,
      workspaceVersion: 2n,
    }),
  } satisfies Pick<ConsoleClient, "features">;
  await assert.rejects(
    featureCollection(client, {
      action: "features",
      project: "p",
      dataset: "d",
      workspace: "w",
      workspaceVersion: "1",
    }),
    /工作区已变化/,
  );
  const result = await featureCollection(client, {
    action: "features",
    project: "p",
    dataset: "d",
    workspace: "w",
  });
  assert.equal(result.numberReturned, 0);
  assert.equal(result.links.length, 1);
  assert.match(result.links[0].href, /workspaceVersion=2/);
  assert.doesNotMatch(result.links[0].href, /revision=/);
});

test("unknown, repeated and invalid query parameters are rejected", () => {
  for (const query of [
    "limit=1&limit=2",
    "endpoint=http://evil",
    "limit=1001",
    "limit=nan",
    "bbox=1,2,3",
    "bbox=1,2,x,4",
    "revision=-1",
    "project=other",
  ]) {
    assert.throws(
      () => featureQuery("p", "d", new URLSearchParams(query)),
      query,
    );
  }
});

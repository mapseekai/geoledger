import assert from "node:assert/strict";
import test from "node:test";
import { type Client, stringifyJson } from "@geoledger/client";
import { featureCollection, featureQuery } from "../src/lib/feature-collection";
import { call } from "../src/lib/browser-api";

test("GeoJSON next link preserves snapshot, bbox and limit without a geometry wrapper", async () => {
  const calls: unknown[] = [];
  const feature = {
    type: "Feature",
    id: "a/b",
    properties: { exact: 18446744073709551615n },
    geometry: null,
  };
  const client = {
    features: async (...args: unknown[]) => {
      calls.push(args);
      return { features: [feature], revision: 9n, nextAfter: "a/b" };
    },
  } as unknown as Client;
  const result = await featureCollection(client, {
    action: "features",
    project: "p",
    dataset: "d",
    limit: 1,
    bbox: [1, 2, 3, 4],
  });
  assert.equal(result.type, "FeatureCollection");
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
      features: [],
      revision: 0n,
      workspaceVersion: 2n,
    }),
  } as unknown as Client;
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

test("browser uses GET FeatureCollection while keeping original integer values for editors", async () => {
  const original = globalThis.fetch;
  try {
    globalThis.fetch = async (url, init) => {
      assert.match(String(url), /^\/api\/projects\/p\/collections\/d\/items/);
      assert.equal(init?.method, "GET");
      return new Response(
        '{"type":"FeatureCollection","features":[{"type":"Feature","id":"one","properties":{"exact":18446744073709551615,"__proto__":{"n":7}},"geometry":null}],"revision":"8","links":[],"numberReturned":1}',
        { headers: { "content-type": "application/geo+json" } },
      );
    };
    const result = await call<{ features: { geojson: string }[] }>({
      action: "features",
      project: "p",
      dataset: "d",
    });
    assert.match(result.features[0].geojson, /18446744073709551615/);
    assert.match(result.features[0].geojson, /"__proto__":/);
  } finally {
    globalThis.fetch = original;
  }
});

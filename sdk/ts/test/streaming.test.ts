"use strict";
const test = require("node:test");
const assert = require("node:assert/strict");
const { Client } = require("../dist");

test("large saves and feature reads use chunked RPCs and reject truncated responses", async () => {
  const grpc = require("@grpc/grpc-js");
  const wire = require("../dist/_internal/geoledger/v1/geoledger");
  const server = new grpc.Server();
  const raw = JSON.stringify({
    type: "Feature",
    id: "large",
    properties: { payload: "x".repeat(5 * 1024 * 1024) },
    geometry: null,
  });
  let uploaded;
  let chunksReceived = 0;
  let truncate = false;
  server.addService(wire.GeoLedgerService, {
    saveStream(call, cb) {
      assert.equal(call.metadata.get("authorization")[0], "Bearer glk_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
      const chunks = [];
      let total;
      call.on("data", (chunk) => {
        if (!chunks.length) total = Number(chunk.totalBytes);
        chunks.push(chunk.data);
        chunksReceived++;
        assert.ok(chunk.data.length <= 65536);
      });
      call.on("end", () => {
        const bytes = Buffer.concat(chunks);
        assert.equal(bytes.length, total);
        uploaded = wire.SaveRequest.decode(bytes);
        cb(null, { version: "1", changes: "1", warnings: [] });
      });
    },
    featuresStream(call) {
      const bytes = wire.FeaturesReply.encode({
        features: [{ geojson: raw }],
        revision: "0",
        workspaceVersion: "1",
      }).finish();
      for (let offset = 0; offset < bytes.length; offset += 65536)
        call.write({
          data: Buffer.from(bytes.subarray(offset, offset + 65536)),
          totalBytes:
            offset === 0 ? String(bytes.length + (truncate ? 1 : 0)) : "0",
        });
      call.end();
    },
  });
  const port = await new Promise((resolve, reject) =>
    server.bindAsync(
      "127.0.0.1:0",
      grpc.ServerCredentials.createInsecure(),
      (error, port) => (error ? reject(error) : resolve(port)),
    ),
  );
  const client = new Client(`http://127.0.0.1:${port}`, "glk_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
  try {
    assert.equal(
      (
        await client.save("p", "w", 0n, [
          { dataset: "d", featureId: "large", feature: raw },
        ])
      ).version,
      1n,
    );
    assert.ok(chunksReceived > 80);
    assert.equal(uploaded.edits[0].feature.geojson, raw);
    const result = await client.features("p", "d");
    assert.equal(result.features[0].properties.payload.length, 5 * 1024 * 1024);
    truncate = true;
    await assert.rejects(
      client.features("p", "d"),
      /invalid streamed response/,
    );
  } finally {
    client.close();
    server.forceShutdown();
  }
});

const { Client, GeoLedgerError, parseJson } = require("../dist");
const assert = require("node:assert/strict");
const { randomUUID } = require("node:crypto");
const fs = require("node:fs");
const token = JSON.parse(fs.readFileSync(process.env.GL_TOKEN_FILE, "utf8"))[0]
  .token;
const client = new Client(
  process.env.GL_ENDPOINT || "http://127.0.0.1:7882",
  token,
);
(async () => {
  try {
    const project = await client.createProject("ts-" + randomUUID());
    const dataset = await client.createDataset(project.id, "places");
    const draft = await client.createWorkspace(project.id);
    const feature = {
      type: "Feature",
      id: "one",
      properties: {
        exact: 18446744073709551615n,
        nested: { geojson: "hello" },
        detail_json: "plain",
      },
      geometry: { type: "Point", coordinates: [1, 2, 3] },
    };
    assert.equal((await draft.save(dataset.id, feature)).version, 1n);
    assert.equal((await draft.save(dataset.id, feature)).version, 2n);
    assert.deepEqual((await draft.features(dataset.id)).features[0], feature);
    await assert.rejects(
      draft.saveBatch([{ dataset: dataset.id, featureId: "one" }]),
      (e) => e instanceof GeoLedgerError && e.code === "invalid_argument",
    );
    await draft.delete(dataset.id, "one");
    assert.deepEqual((await draft.features(dataset.id)).features, []);
    await draft.save(dataset.id, feature);
    assert.equal(draft.info.version, 4n);
    const receipt = await draft.publish("TypeScript SDK");
    assert.deepEqual(await draft.publish("TypeScript SDK"), receipt);
    assert.deepEqual(
      (
        await client.features(project.id, dataset.id, {
          revision: receipt.revision,
        })
      ).features[0],
      feature,
    );
    assert.equal((await client.history(project.id)).length, 1);
    assert.equal(
      (await client.commit(project.id, receipt.revision)).changes.length,
      1,
    );
    assert.ok((await client.audit(project.id)).events.length);
    await assert.rejects(
      client.publish({ ...draft.pendingPublication, message: "different" }),
      (e) =>
        e instanceof GeoLedgerError && e.code === "conflict" && !e.uncertain,
    );
    await assert.rejects(
      client.createDataset(project.id, "places"),
      (e) => e.code === "conflict",
    );
    assert.equal(parseJson("9007199254740993.0"), 9007199254740993n);
    console.log(
      "TypeScript SDK: business promises, bigint GeoJSON, automatic draft versions, publish retry OK",
    );
  } finally {
    client.close();
  }
})().catch((e) => {
  console.error(e);
  process.exitCode = 1;
});

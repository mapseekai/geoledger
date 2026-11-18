"use strict";
const test = require("node:test");
const assert = require("node:assert/strict");
const { Client, Workspace, GeoLedgerError, parseJson } = require("../dist");

test("registerTable preserves the CreateDataset wire contract and permission errors", async () => {
  const wire = require("../dist/_internal/geoledger/v1/geoledger");
  const grpc = require("@grpc/grpc-js");
  const source = {
    schema: "business",
    table: "roads",
    idColumn: "road_id",
    geometryColumn: "shape",
  };
  const sent = [];
  let denied = false;
  const original = wire.GeoLedgerClient.prototype.createDataset;
  wire.GeoLedgerClient.prototype.createDataset = (request, callback) => {
    sent.push(request);
    if (denied) {
      callback(
        Object.assign(new Error("registration denied"), {
          code: grpc.status.PERMISSION_DENIED,
        }),
      );
    } else {
      callback(null, {
        dataset: "dataset",
        name: request.name,
        geometryType: "line",
        coordinateDimension: 3,
        crs: "EPSG:3857",
        postgisTable: source,
      });
    }
  };
  const client = new Client(
    "http://127.0.0.1:7882",
    "glk_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
  );
  try {
    assert.deepEqual(await client.registerTable("project", "roads", source), {
      id: "dataset",
      name: "roads",
      geometryType: "line",
      coordinateDimension: 3,
      crs: "EPSG:3857",
      postgisTable: source,
    });
    assert.deepEqual(sent, [
      {
        project: "project",
        name: "roads",
        geometryType: "",
        coordinateDimension: 0,
        crs: "",
        postgisTable: source,
      },
    ]);
    denied = true;
    await assert.rejects(
      client.registerTable("project", "roads", source),
      (error) =>
        error instanceof GeoLedgerError &&
        error.code === "permission_denied" &&
        !error.uncertain,
    );
  } finally {
    client.close();
    wire.GeoLedgerClient.prototype.createDataset = original;
  }
});

test("plaintext to remote hosts requires explicit opt-in", () => {
  const { Client, isLoopbackHost } = require("../dist/index.js");
  assert.throws(
    () => new Client("http://geoledger.example:7882", "glk_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
    (e) => e.code === "invalid_argument",
  );
  new Client("http://geoledger.example:7882", "glk_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", 1000, {
    allowInsecure: true,
  }).close();
  new Client("http://127.0.0.1:7882", "glk_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").close();
  assert.ok(
    isLoopbackHost("[::1]") &&
      isLoopbackHost("localhost") &&
      !isLoopbackHost("10.0.0.1"),
  );
});

test("public client keeps generated transport private", () => {
  const client = new Client("http://127.0.0.1:7882", "glk_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
  try {
    assert.equal("rpc" in client, false);
    assert.equal("GeoLedgerClient" in require("../dist"), false);
  } finally {
    client.close();
  }
});

test("JSON properties remain opaque and integer tokens remain exact", () => {
  const parsed = parseJson(
    '{"nested":{"geojson":"ordinary text"},"detail_json":"plain","exact":9007199254740993,"max":18446744073709551615}',
  );
  assert.deepEqual(parsed.nested, { geojson: "ordinary text" });
  assert.equal(parsed.detail_json, "plain");
  assert.equal(parsed.exact, 9007199254740993n);
  assert.equal(parsed.max, 18446744073709551615n);
});
test("JSON preserves own __proto__ properties without rounding numbers", () => {
  const parsed = parseJson(
    '{"__proto__":{"exact":18446744073709551615},"ordinary":9007199254740993}',
  );
  assert.equal(Object.hasOwn(parsed, "__proto__"), true);
  assert.equal(Object.getPrototypeOf(parsed), Object.prototype);
  assert.equal(parsed.__proto__.exact, 18446744073709551615n);
  assert.equal(parsed.ordinary, 9007199254740993n);
  const escaped = parseJson('{"__pr\\u006fto__":18446744073709551615}');
  assert.equal(Object.hasOwn(escaped, "__proto__"), true);
  assert.equal(escaped.__proto__, 18446744073709551615n);
});

test("integral decimal and exponent spellings must not silently lose digits", () => {
  for (const raw of ["9007199254740993.0", "90071992547409930e-1"]) {
    // Rejection is safe; returning a rounded Number is not.
    let value;
    try {
      value = parseJson(raw);
    } catch {
      continue;
    }
    assert.equal(value, 9007199254740993n);
  }
  assert.throws(() => parseJson("1e-400"));
});

test("missing feature rejects locally and explicit deletion is supported", async () => {
  const client = new Client("http://127.0.0.1:7882", "glk_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
  try {
    await assert.rejects(
      client.save("project", "draft", 0n, [
        { dataset: "dataset", featureId: "one" },
      ]),
      (e) => e instanceof GeoLedgerError && e.code === "invalid_argument",
    );
  } finally {
    client.close();
  }
  const sent = [];
  const workspace = new Workspace(
    {
      save: async (...args) => {
        sent.push(args);
        return { version: 1n, changes: 1n };
      },
    },
    "project",
    { id: "draft", baseRevision: 0n, version: 0n, status: "open" },
  );
  await workspace.delete("dataset", "one");
  assert.equal(sent[0][3][0].feature, null);
  assert.equal(workspace.info.version, 1n);
});

test("one workspace rejects concurrent mutation and advances its version", async () => {
  let complete;
  const sent = [];
  const workspace = new Workspace(
    {
      save: (...args) => {
        sent.push(args);
        return new Promise((resolve) => {
          complete = resolve;
        });
      },
    },
    "project",
    { id: "draft", baseRevision: 0n, version: 0n, status: "open" },
  );
  const first = workspace.delete("dataset", "one");
  await assert.rejects(
    workspace.delete("dataset", "two"),
    /already in progress/,
  );
  assert.equal(sent.length, 1);
  complete({ version: 1n, changes: 1n });
  await first;
  assert.equal(workspace.info.version, 1n);
  assert.equal(Object.isFrozen(workspace.info), true);
});

test("uncertain publication preserves its immutable request and blocks edits", async () => {
  const sent = [];
  const client = {
    publish: async (intent) => {
      sent.push({ ...intent });
      if (sent.length === 1)
        throw new GeoLedgerError(
          "unavailable",
          "response lost",
          undefined,
          undefined,
          true,
        );
      return {
        revision: 2n,
        workspace: "draft",
        version: 5n,
        status: "published",
        changes: 1n,
      };
    },
  };
  const workspace = new Workspace(client, "project", {
    id: "draft",
    baseRevision: 1n,
    version: 4n,
    status: "open",
  });
  await assert.rejects(workspace.publish("roads updated"), (e) => e.uncertain);
  assert.equal(Object.isFrozen(workspace.pendingPublication), true);
  assert.throws(() => {
    workspace.pendingPublication.message = "changed";
  }, TypeError);
  await assert.rejects(
    workspace.publish("different message"),
    /original message/,
  );
  await assert.rejects(
    workspace.delete("dataset", "one"),
    /pending publication/,
  );
  assert.equal(sent.length, 1);
  await workspace.publish("roads updated");
  assert.deepEqual(sent[0], sent[1]);
  assert.equal(workspace.info.version, 5n);
  assert.equal(workspace.info.status, "published");
});

test("definite conflict permits resolution with the unchanged draft version", async () => {
  const workspace = new Workspace(
    {
      publish: async () => {
        throw new GeoLedgerError("conflict", "merge conflicts");
      },
    },
    "project",
    { id: "draft", baseRevision: 1n, version: 4n, status: "open" },
  );
  await assert.rejects(
    workspace.publish("roads updated"),
    (e) => e.code === "conflict",
  );
  assert.equal(workspace.pendingPublication, undefined);
  assert.equal(workspace.info.version, 4n);
});

for (const code of ["unauthenticated", "permission_denied", "not_found", "invalid_argument", "conflict", "unknown_business_error"]) {
  test(`retry ${code} preserves original publication`, async () => {
    const sent = [];
    const workspace = new Workspace(
      {
        publish: async (intent) => {
          sent.push({ ...intent });
          if (sent.length === 1)
            throw new GeoLedgerError(
              "unavailable",
              "lost",
              undefined,
              undefined,
              true,
            );
          if (sent.length === 2)
            throw new GeoLedgerError(code, "access denied");
          return { revision: 2n, version: 5n, status: "published" };
        },
      },
      "project",
      { id: "draft", baseRevision: 1n, version: 4n, status: "open" },
    );
    await assert.rejects(workspace.publish("original"));
    await assert.rejects(workspace.publish("original"));
    assert.deepEqual(workspace.pendingPublication, sent[0]);
    await assert.rejects(workspace.publish("changed"));
    await workspace.publish("original");
    assert.deepEqual(sent, [sent[0], sent[0], sent[0]]);
  });
}

test("first access failure and permanent workspace version rejection release intent", async () => {
  for (const code of [
    "unauthenticated",
    "permission_denied",
    "not_found",
    "invalid_argument",
    "workspace_version_conflict",
  ]) {
    let calls = 0;
    const workspace = new Workspace(
      {
        publish: async () => {
          if (code === "workspace_version_conflict" && calls++ === 0)
            throw new GeoLedgerError(
              "unavailable",
              "lost",
              undefined,
              undefined,
              true,
            );
          throw new GeoLedgerError(code, "rejected");
        },
      },
      "project",
      { id: "draft", baseRevision: 1n, version: 4n, status: "open" },
    );
    if (code === "workspace_version_conflict")
      await assert.rejects(workspace.publish("original"));
    await assert.rejects(workspace.publish("original"));
    assert.equal(workspace.pendingPublication, undefined);
  }
});

test("source rejection is definite for both client levels and permits correction", async () => {
  const wire = require("../dist/_internal/geoledger/v1/geoledger");
  const grpc = require("@grpc/grpc-js");
  const metadata = new grpc.Metadata();
  metadata.set("grpc-status-details-bin", Buffer.from(wire.RpcStatus.encode({
    code: grpc.status.INVALID_ARGUMENT,
    message: "source rejected",
    details: [{
      typeUrl: "type.googleapis.com/geoledger.v1.ErrorDetail",
      value: wire.ErrorDetail.encode({ code: "publication_attempt_rejected", message: "source rejected", requestId: "request" }).finish(),
    }],
  }).finish()));
  const sent = [];
  const original = wire.GeoLedgerClient.prototype.publish;
  wire.GeoLedgerClient.prototype.publish = (request, callback) => {
    sent.push(request);
    callback(Object.assign(new Error("source rejected"), { code: grpc.status.INVALID_ARGUMENT, metadata }));
  };
  const client = new Client("http://127.0.0.1:7882", "glk_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
  try {
    const intent = { project: "project", workspace: "draft", expectedWorkspaceVersion: 4n, requestId: "request", message: "original" };
    await assert.rejects(client.publish(intent), (e) => e.code === "publication_attempt_rejected" && !e.uncertain);
    const workspace = new Workspace(client, "project", { id: "draft", baseRevision: 1n, version: 4n, status: "open" });
    await assert.rejects(workspace.publish("original"), (e) => e.code === "publication_attempt_rejected" && !e.uncertain && e.requestId === "request");
    assert.equal(workspace.pendingPublication, undefined);
    assert.equal(workspace.info.version, 4n);
    let saves = 0;
    client.save = async () => { saves++; return { version: 5n, changes: 1n }; };
    client.publish = async (next) => {
      assert.notEqual(next.requestId, sent[1].requestId);
      assert.equal(next.expectedWorkspaceVersion, 5n);
      return { revision: 2n, version: 6n, status: "published" };
    };
    await workspace.delete("dataset", "one");
    await workspace.publish("corrected");
    assert.equal(saves, 1);
  } finally {
    client.close();
    wire.GeoLedgerClient.prototype.publish = original;
  }
});

for (const uncertain of [false, true]) {
  test(`source rejection uncertain=${uncertain} preserves an earlier unknown attempt`, async () => {
    const sent = [];
    const workspace = new Workspace({
      publish: async (intent) => {
        sent.push({ ...intent });
        if (sent.length === 1)
          throw new GeoLedgerError("unavailable", "old attempt still queued", undefined, undefined, true);
        if (sent.length === 2)
          throw new GeoLedgerError("publication_attempt_rejected", "retry rejected", undefined, undefined, uncertain);
        return { revision: 2n, version: 5n, status: "published" };
      },
    }, "project", { id: "draft", baseRevision: 1n, version: 4n, status: "open" });
    await assert.rejects(workspace.publish("original"));
    await assert.rejects(workspace.publish("original"));
    assert.deepEqual(workspace.pendingPublication, sent[0]);
    await assert.rejects(workspace.delete("dataset", "one"), /pending publication/);
    await workspace.publish("original");
    assert.deepEqual(sent, [sent[0], sent[0], sent[0]]);
  });
}

test("schema query and save preserve exact versions and omitted versus empty intent", async () => {
  const wire = require("../dist/_internal/geoledger/v1/geoledger");
  const sent = [];
  const originalSave = wire.GeoLedgerClient.prototype.save;
  const originalSchema = wire.GeoLedgerClient.prototype.schema;
  wire.GeoLedgerClient.prototype.save = (request, callback) => {
    sent.push(wire.SaveRequest.decode(wire.SaveRequest.encode(request).finish()));
    callback(null, {version:String(BigInt(request.expectedWorkspaceVersion)+1n), changes:"1", warnings:[]});
  };
  wire.GeoLedgerClient.prototype.schema = (request, callback) => callback(null, {
    revision:request.revision??"1", workspaceVersion:request.workspace===undefined?undefined:"4",
    fields:[{name:"id",type:"int8",nullable:false,editable:false}],
  });
  const client = new Client("http://127.0.0.1:7882", "glk_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
  try {
    const workspace = new Workspace(client,"p",{id:"w",version:4n,baseRevision:1n,status:"open"});
    await workspace.saveChanges([],[{dataset:"d",add:[{name:"note",type:"string"}]}]);
    await workspace.saveBatch([]);
    await workspace.saveChanges([],[]);
    assert.equal(sent[0].schemaEdits.edits[0].add[0].type,"string");
    assert.equal(sent[1].schemaEdits,undefined);
    assert.deepEqual(sent[2].schemaEdits.edits,[]);
    assert.deepEqual(sent.map(r=>r.expectedWorkspaceVersion),["4","5","6"]);
    assert.equal((await workspace.schema("d")).workspaceVersion,4n);
    const result=await client.schema("p","d",{revision:9007199254740993n});
    assert.equal(result.revision,9007199254740993n);
    assert.equal(result.workspaceVersion,undefined);
    assert.equal(result.fields[0].editable,false);
  } finally {
    client.close();
    wire.GeoLedgerClient.prototype.save=originalSave;
    wire.GeoLedgerClient.prototype.schema=originalSchema;
  }
});

test("deadline and range transport failures use canonical codes and retain publication intent", async () => {
  const wire = require("../dist/_internal/geoledger/v1/geoledger");
  const grpc = require("@grpc/grpc-js");
  const original = wire.GeoLedgerClient.prototype.publish;
  try {
    for (const [status, fallback] of [
      [grpc.status.DEADLINE_EXCEEDED, "deadline_exceeded"],
      [grpc.status.OUT_OF_RANGE, "resource_exhausted"],
    ]) {
      for (const businessCode of [undefined, "business_range"]) {
        const metadata = new grpc.Metadata();
        if (businessCode) {
          metadata.set("grpc-status-details-bin", Buffer.from(wire.RpcStatus.encode({
            code: status,
            message: "transport failure",
            details: [{
              typeUrl: "type.googleapis.com/geoledger.v1.ErrorDetail",
              value: wire.ErrorDetail.encode({ code: businessCode, message: "business rejection", requestId: "request" }).finish(),
            }],
          }).finish()));
        }
        const sent = [];
        wire.GeoLedgerClient.prototype.publish = (request, callback) => {
          sent.push({ ...request });
          callback(Object.assign(new Error("transport failure"), { code: status, metadata }));
        };
        const client = new Client("http://127.0.0.1:7882", "glk_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        try {
          const workspace = new Workspace(client, "project", { id: "draft", baseRevision: 1n, version: 4n, status: "open" });
          for (let attempt = 0; attempt < 2; attempt++) {
            await assert.rejects(workspace.publish("original"), (e) =>
              e instanceof GeoLedgerError && e.code === (businessCode ?? fallback) && e.uncertain,
            );
          }
          assert(workspace.pendingPublication);
          assert.deepEqual(sent[0], sent[1]);
          await assert.rejects(workspace.delete("dataset", "one"), /pending publication/);
        } finally {
          client.close();
        }
      }
    }
  } finally {
    wire.GeoLedgerClient.prototype.publish = original;
  }
});


test("conflict-family statuses require a matching current business detail", async () => {
  const wire = require("../dist/_internal/geoledger/v1/geoledger");
  const grpc = require("@grpc/grpc-js");
  const original = wire.GeoLedgerClient.prototype.publish;
  try {
    for (const status of [grpc.status.ABORTED, grpc.status.ALREADY_EXISTS, grpc.status.FAILED_PRECONDITION]) {
      for (const kind of ["absent", "malformed", "mismatched", "empty code", "wrong type"]) {
        const metadata = new grpc.Metadata();
        if (kind !== "absent") {
          const raw = kind === "malformed" ? Buffer.from([255]) : Buffer.from(wire.RpcStatus.encode({
            code: kind === "mismatched" ? grpc.status.UNAVAILABLE : status,
            message: "proxy rejection",
            details: [{
              typeUrl: kind === "wrong type" ? "type.invalid/geoledger.v1.ErrorDetail" : "type.googleapis.com/geoledger.v1.ErrorDetail",
              value: wire.ErrorDetail.encode({ code: kind === "empty code" ? "" : "workspace_version_conflict", message: "not trusted", requestId: "request" }).finish(),
            }],
          }).finish());
          metadata.set("grpc-status-details-bin", raw);
        }
        const sent = [];
        wire.GeoLedgerClient.prototype.publish = (request, callback) => {
          sent.push({ ...request });
          callback(Object.assign(new Error("proxy rejection"), { code: status, metadata }));
        };
        const client = new Client("http://127.0.0.1:7882", "glk_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        try {
          const workspace = new Workspace(client, "project", { id: "draft", baseRevision: 1n, version: 4n, status: "open" });
          for (let attempt = 0; attempt < 2; attempt++) await assert.rejects(workspace.publish("original"), (e) => e.code === "conflict" && e.uncertain);
          const intent = workspace.pendingPublication;
          assert(intent);
          assert.deepEqual(sent[0], sent[1]);
          await assert.rejects(workspace.delete("dataset", "one"), /pending publication/);
          client.publish = async (recovered) => {
            assert.deepEqual(recovered, intent);
            return { revision: 2n, version: 5n, status: "published", changes: 1n };
          };
          await workspace.publish("original");
          assert.equal(workspace.info.status, "published");
        } finally { client.close(); }
      }
    }
  } finally { wire.GeoLedgerClient.prototype.publish = original; }
});

test("workspace version proofs require Aborted and complete non-merge details", async () => {
  const wire = require("../dist/_internal/geoledger/v1/geoledger");
  const grpc = require("@grpc/grpc-js");
  const original = wire.GeoLedgerClient.prototype.publish;
  const cases = [
    { name: "valid", status: grpc.status.ABORTED, valid: true },
    ...[grpc.status.PERMISSION_DENIED, grpc.status.INVALID_ARGUMENT, grpc.status.ALREADY_EXISTS, grpc.status.FAILED_PRECONDITION, grpc.status.UNAVAILABLE].map((status) => ({ name: `status ${status}`, status, valid: false })),
    { name: "empty message", status: grpc.status.ABORTED, message: "", valid: false },
    { name: "blank message", status: grpc.status.ABORTED, message: " \t", valid: false },
    { name: "empty request", status: grpc.status.ABORTED, requestId: "", valid: false },
    { name: "blank request", status: grpc.status.ABORTED, requestId: " \t", valid: false },
    { name: "merge metadata", status: grpc.status.ABORTED, conflicts: {}, valid: false },
  ];
  try {
    for (const item of cases) {
      let failureStatus = grpc.status.UNAVAILABLE;
      let metadata = new grpc.Metadata();
      wire.GeoLedgerClient.prototype.publish = (_request, callback) => callback(Object.assign(new Error("public failure"), { code: failureStatus, metadata }));
      const client = new Client("http://127.0.0.1:7882", "glk_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
      try {
        const workspace = new Workspace(client, "project", { id: "draft", baseRevision: 1n, version: 4n, status: "open" });
        await assert.rejects(workspace.publish("original"), (e) => e.uncertain);
        const intent = workspace.pendingPublication;
        assert(intent);
        const detail = wire.ErrorDetail.fromPartial({ code: "workspace_version_conflict", message: "workspace version changed", requestId: "proof", ...item });
        metadata = new grpc.Metadata();
        metadata.set("grpc-status-details-bin", Buffer.from(wire.RpcStatus.encode({ code: item.status, message: "public failure", details: [{ typeUrl: "type.googleapis.com/geoledger.v1.ErrorDetail", value: wire.ErrorDetail.encode(detail).finish() }] }).finish()));
        failureStatus = item.status;
        await assert.rejects(workspace.publish("original"), (e) => e.code === "workspace_version_conflict" && e.uncertain === !item.valid);
        if (item.valid) assert.equal(workspace.pendingPublication, undefined, item.name);
        else assert.deepEqual(workspace.pendingPublication, intent, item.name);
      } finally { client.close(); }
    }
  } finally { wire.GeoLedgerClient.prototype.publish = original; }
});

test("a malformed publication receipt stays uncertain until the exact original receipt arrives", async () => {
  const wire = require("../dist/_internal/geoledger/v1/geoledger");
  const original = wire.GeoLedgerClient.prototype.publish;
  const valid = { workspace: "draft", status: "published", revision: "2", version: "5", changes: "0" };
  try {
    for (const bad of [{}, { ...valid, workspace: "other" }, { ...valid, version: "4" }, { ...valid, status: "open" }, { ...valid, revision: "0" }, { ...valid, changes: "-1" }]) {
      let receipt = wire.PublishReply.fromPartial(bad);
      const sent = [];
      wire.GeoLedgerClient.prototype.publish = (request, callback) => { sent.push({ ...request }); callback(null, receipt); };
      const client = new Client("http://127.0.0.1:7882", "glk_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
      try {
        const workspace = new Workspace(client, "project", { id: "draft", baseRevision: 1n, version: 4n, status: "open" });
        await assert.rejects(workspace.publish("original"), (e) => e.code === "invalid_response" && e.uncertain);
        const intent = workspace.pendingPublication;
        assert(intent);
        assert.equal(workspace.info.version, 4n);
        assert.equal(workspace.info.status, "open");
        await assert.rejects(workspace.delete("dataset", "one"), /pending publication/);
        receipt = wire.PublishReply.fromPartial(valid);
        const result = await workspace.publish("original");
        assert.equal(result.changes, 0n);
        assert.equal(workspace.info.status, "published");
        assert.deepEqual(sent[0], sent[1]);
        assert.deepEqual(workspace.pendingPublication, intent);
      } finally { client.close(); }
    }
  } finally { wire.GeoLedgerClient.prototype.publish = original; }
});

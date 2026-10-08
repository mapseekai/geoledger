"use strict";
const test = require("node:test");
const assert = require("node:assert/strict");
const { Client, Workspace, GeoLedgerError, parseJson } = require("../dist");

test("public client keeps generated transport private", () => {
  const client = new Client("http://127.0.0.1:7882", "test-only-token");
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
  const client = new Client("http://127.0.0.1:7882", "test-only-token");
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

for (const code of ["unauthenticated", "permission_denied", "not_found"]) {
  test(`retry ${code} preserves original publication`, async () => {
    const sent = [];
    const workspace = new Workspace({publish: async (intent) => {
      sent.push({...intent});
      if (sent.length === 1) throw new GeoLedgerError("unavailable", "lost", undefined, undefined, true);
      if (sent.length === 2) throw new GeoLedgerError(code, "access denied");
      return {revision: 2n, version: 5n, status: "published"};
    }}, "project", {id: "draft", baseRevision: 1n, version: 4n, status: "open"});
    await assert.rejects(workspace.publish("original"));
    await assert.rejects(workspace.publish("original"));
    assert.deepEqual(workspace.pendingPublication, sent[0]);
    await assert.rejects(workspace.publish("changed"));
    await workspace.publish("original");
    assert.deepEqual(sent, [sent[0], sent[0], sent[0]]);
  });
}

test("first access failure and retry conflict release intent", async () => {
  for (const code of ["unauthenticated", "permission_denied", "not_found", "conflict"]) {
    let calls = 0;
    const workspace = new Workspace({publish: async () => {
      if (code === "conflict" && calls++ === 0)
        throw new GeoLedgerError("unavailable", "lost", undefined, undefined, true);
      throw new GeoLedgerError(code, "rejected");
    }}, "project", {id: "draft", baseRevision: 1n, version: 4n, status: "open"});
    if (code === "conflict") await assert.rejects(workspace.publish("original"));
    await assert.rejects(workspace.publish("original"));
    assert.equal(workspace.pendingPublication, undefined);
  }
});

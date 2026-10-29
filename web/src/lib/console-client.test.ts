import test from "node:test";
import assert from "node:assert/strict";
import { consoleClient, authRequest } from "./console-client";
import { execute, encode } from "./operations";
import { stringifyJson, GeoLedgerError } from "@geoledger/client";

test("private transport sends only Session header and preserves exact business metadata and JSON", async () => {
  const savedFetch = globalThis.fetch;
  const savedSecret = process.env.GL_WEB_SESSION_SECRET;
  const savedEndpoint = process.env.GL_WEB_ENDPOINT;
  process.env.GL_WEB_SESSION_SECRET = "test-only-session-secret-".repeat(3);
  process.env.GL_WEB_ENDPOINT = "http://127.0.0.1:18881";
  const requests: { url: string; body: string; headers: Headers }[] = [];
  let response = "{}";
  let status = 200;
  globalThis.fetch = async (input, init) => {
    requests.push({
      url: String(input),
      body: String(init?.body),
      headers: new Headers(init?.headers),
    });
    return new Response(response, { status });
  };
  try {
    const client = consoleClient("test-session");
    response =
      '{"project":"p","name":"test","head":9007199254740993,"role":"owner","state":"active"}';
    const project = await client.project("p");
    assert.equal(project.head, 9007199254740993n);
    assert.equal(project.id, "p");
    assert.match(requests[0].url, /api\/v1\/get_project$/);
    assert.equal(
      requests[0].headers.get("x-geoledger-session"),
      "test-session",
    );
    assert.equal(requests[0].headers.has("authorization"), false);
    response = '{"revision":9007199254740994,"version":1,"changes":1}';
    const publish = {
      action: "publish",
      project: "p",
      workspace: "w",
      version: "9007199254740993",
      requestId: "733fcf66-a9a6-4636-9496-cc7d831f7789",
      message: "m",
    };
    assert.match(
      encode(await execute(client, publish)),
      /"revision":"9007199254740994"/,
    );
    assert.match(
      requests.at(-1)!.body,
      /"expected_workspace_version":9007199254740993/,
    );
    response =
      '{"type":"FeatureCollection","crs":"EPSG:4326","revision":2,"workspace_version":null,"next_after":null,"features":[{"type":"Feature","id":"f","properties":{"snake_case":18446744073709551615,"exact":1.00000000000000001e18,"__proto__":{"safe":1},"numericLike":{"isLosslessNumber":true,"value":"7"}},"geometry":{"type":"LineString","coordinates":[[104,35],[106,37]]}}]}';
    const features = await execute(client, {
      action: "features",
      project: "p",
      dataset: "d",
    });
    const raw = stringifyJson(features);
    assert.match(raw, /"snake_case":18446744073709551615/);
    assert.match(raw, /"exact":1.00000000000000001e18/);
    assert.match(raw, /"__proto__":/);
    assert.match(raw, /"numericLike":\{"isLosslessNumber":true,"value":"7"\}/);
    assert.match(raw, /"coordinates":\[\[104,35\],\[106,37\]\]/);
    response =
      '{"commits":[{"revision":9007199254740993}],"next_after":9007199254740993}';
    assert.equal((await client.history("p")).nextAfter, 9007199254740993n);
    response = '{"commits":[{"revision":9007199254740993}],"next_after":null}';
    assert.equal((await client.history("p")).nextAfter, undefined);
    response =
      '{"base_revision":1,"version":2,"changes":[{"cursor":"d/key/1"}],"next_after":"d/key/1"}';
    assert.equal((await client.diff("p", "w")).nextAfter, "d/key/1");
    response = '{"revision":1,"changes":[],"next_after":null}';
    assert.equal((await client.commit("p", 1n)).nextAfter, undefined);
    response =
      '{"events":[{"id":9007199254740993,"detail":{"next_after":"opaque"}}],"next_after":null}';
    const audit = await client.audit("p");
    assert.equal(audit.nextAfter, undefined);
    assert.equal(audit.events[0]?.id, 9007199254740993n);
    assert.deepEqual(audit.events[0]?.detail, { next_after: "opaque" });
    response = '{"version":3,"changes":1}';
    await execute(client, {
      action: "save",
      project: "p",
      workspace: "w",
      version: "2",
      edits: [
        {
          dataset: "d",
          featureId: "f",
          feature:
            '{"type":"Feature","id":"f","properties":{"exact":1.00000000000000001e18,"big":18446744073709551615,"numericLike":{"isLosslessNumber":true,"value":"7"}},"geometry":{"type":"LineString","coordinates":[[104,35],[106,37]]}}',
        },
      ],
    });
    assert.match(requests.at(-1)!.body, /"feature_id":"f"/);
    assert.match(requests.at(-1)!.body, /"exact":1.00000000000000001e18/);
    assert.match(requests.at(-1)!.body, /"big":18446744073709551615/);
    assert.match(
      requests.at(-1)!.body,
      /"numericLike":\{"isLosslessNumber":true,"value":"7"\}/,
    );
    assert.match(
      requests.at(-1)!.body,
      /"coordinates":\[\[104,35\],\[106,37\]\]/,
    );
    await execute(client, {
      action: "setMember",
      project: "p",
      subject: "stable-user-id",
      role: "viewer",
      currentPassword: "test-only-password",
    });
    assert.match(
      requests.at(-1)!.body,
      /"current_password":"test-only-password"/,
    );
    status = 409;
    response =
      '{"error":"merge conflicts","head":9007199254740993,"version":4,"total":1,"truncated":false,"conflicts":[{"cursor":"d/f","dataset":"d","feature_id":"f","fields":[],"base":null,"current":{"type":"Feature","id":"f","properties":{"exact":18446744073709551615},"geometry":null},"draft":null}]}';
    await assert.rejects(client.conflicts("p", "w"), (error) => {
      assert(error instanceof GeoLedgerError);
      assert.equal(error.conflicts?.head, 9007199254740993n);
      assert.equal(error.conflicts?.conflicts[0].featureId, "f");
      assert.match(stringifyJson(error.conflicts), /18446744073709551615/);
      return true;
    });
    status = 429;
    response =
      '{"error":{"code":"busy","message":"too many attempts","request_id":"safe-request-id"}}';
    await assert.rejects(authRequest("me", {}, "test-session"), (error) => {
      assert(error instanceof GeoLedgerError);
      assert.equal(error.code, "busy");
      assert.equal(error.requestId, "safe-request-id");
      assert.equal((error as GeoLedgerError & { status: number }).status, 429);
      return true;
    });
    status = 200;
    response = '{"session":"opaque","user":{"username":"admin"}}';
    await authRequest("login_password", {
      username: "admin",
      password: "test-only",
    });
    assert.match(requests.at(-1)!.url, /auth\/v1\/login_password$/);
    assert.equal(requests.at(-1)!.headers.has("x-geoledger-session"), false);
  } finally {
    globalThis.fetch = savedFetch;
    if (savedSecret === undefined) delete process.env.GL_WEB_SESSION_SECRET;
    else process.env.GL_WEB_SESSION_SECRET = savedSecret;
    if (savedEndpoint === undefined) delete process.env.GL_WEB_ENDPOINT;
    else process.env.GL_WEB_ENDPOINT = savedEndpoint;
  }
});

test("table registration keeps the private create_dataset request and Session credentials", async () => {
  const savedFetch = globalThis.fetch;
  const savedSecret = process.env.GL_WEB_SESSION_SECRET;
  const savedEndpoint = process.env.GL_WEB_ENDPOINT;
  process.env.GL_WEB_SESSION_SECRET = "test-only-session-secret-".repeat(3);
  process.env.GL_WEB_ENDPOINT = "http://127.0.0.1:18881";
  const source = {
    schema: "business",
    table: "roads",
    idColumn: "road_id",
    geometryColumn: "shape",
  };
  globalThis.fetch = async (input, init) => {
    assert.equal(String(input), "http://127.0.0.1:18881/api/v1/create_dataset");
    assert.deepEqual(JSON.parse(String(init?.body)), {
      project: "p",
      name: "roads",
      postgis_table: {
        schema: "business",
        table: "roads",
        id_column: "road_id",
        geometry_column: "shape",
      },
    });
    const headers = new Headers(init?.headers);
    assert.equal(headers.get("x-geoledger-session"), "registration-session");
    assert.equal(headers.has("authorization"), false);
    return new Response(
      JSON.stringify({
        dataset: "d",
        name: "roads",
        geometry_type: "line",
        coordinate_dimension: 3,
        crs: "EPSG:3857",
        postgis_table: {
          schema: "business",
          table: "roads",
          id_column: "road_id",
          geometry_column: "shape",
        },
      }),
    );
  };
  try {
    assert.deepEqual(
      await consoleClient("registration-session").registerTable(
        "p",
        "roads",
        source,
      ),
      {
        dataset: "d",
        id: "d",
        name: "roads",
        geometryType: "line",
        coordinateDimension: 3,
        crs: "EPSG:3857",
        postgisTable: source,
      },
    );
  } finally {
    globalThis.fetch = savedFetch;
    if (savedSecret === undefined) delete process.env.GL_WEB_SESSION_SECRET;
    else process.env.GL_WEB_SESSION_SECRET = savedSecret;
    if (savedEndpoint === undefined) delete process.env.GL_WEB_ENDPOINT;
    else process.env.GL_WEB_ENDPOINT = savedEndpoint;
  }
});

test("commit pagination cannot overwrite the normalized exact revision", async () => {
  const originalFetch = globalThis.fetch;
  const originalSecret = process.env.GL_WEB_SESSION_SECRET;
  process.env.GL_WEB_SESSION_SECRET = "test-only-session-secret-".repeat(3);
  let body = "";
  globalThis.fetch = async (_input, init) => {
    body = String(init?.body);
    return new Response(
      '{"revision":9007199254740993,"changes":[],"next_after":null}',
    );
  };
  try {
    await execute(consoleClient("test-session"), {
      action: "commit",
      project: "p",
      revision: "9007199254740993",
      after: "dataset/feature",
      limit: 50,
    });
    assert.match(body, /"revision":9007199254740993/);
    assert.match(body, /"after":"dataset\/feature"/);
    assert.match(body, /"limit":50/);
    assert.doesNotMatch(body, /"action"/);
  } finally {
    globalThis.fetch = originalFetch;
    if (originalSecret === undefined) delete process.env.GL_WEB_SESSION_SECRET;
    else process.env.GL_WEB_SESSION_SECRET = originalSecret;
  }
});

test("resolve and rebase send canonical resolutions with exact concurrency guards", async () => {
  const originalFetch = globalThis.fetch;
  const originalSecret = process.env.GL_WEB_SESSION_SECRET;
  process.env.GL_WEB_SESSION_SECRET = "test-only-session-secret-".repeat(3);
  const bodies: string[] = [];
  globalThis.fetch = async (_input, init) => {
    bodies.push(String(init?.body));
    return new Response(
      '{"head":2,"base_revision":2,"version":3,"remaining_conflicts":0,"changes":1}',
    );
  };
  try {
    for (const action of ["resolve", "rebase"] as const) {
      await execute(consoleClient("test-session"), {
        action,
        project: "p",
        workspace: "w",
        version: "9007199254740993",
        head: "2",
        edits: [
          {
            dataset: "d",
            featureId: "f",
            feature:
              '{"type":"Feature","id":"f","properties":{"exact":18446744073709551615},"geometry":null}',
          },
        ],
      });
      const body = bodies.at(-1)!;
      assert.match(body, /"resolutions":\[/);
      assert.doesNotMatch(body, /"edits":/);
      assert.match(body, /"expected_workspace_version":9007199254740993/);
      assert.match(body, /"expected_head":2/);
      assert.match(body, /"feature_id":"f"/);
      assert.match(body, /"exact":18446744073709551615/);
    }
  } finally {
    globalThis.fetch = originalFetch;
    if (originalSecret === undefined) delete process.env.GL_WEB_SESSION_SECRET;
    else process.env.GL_WEB_SESSION_SECRET = originalSecret;
  }
});

test("HTTP admission and timeout errors match SDK uncertainty and fallback codes", async () => {
  const originalFetch = globalThis.fetch;
  const originalSecret = process.env.GL_WEB_SESSION_SECRET;
  process.env.GL_WEB_SESSION_SECRET = "test-only-session-secret-".repeat(3);
  try {
    for (const [status, code] of [
      [413, "resource_exhausted"],
      [429, "resource_exhausted"],
      [408, "timeout"],
      [504, "timeout"],
      [499, "cancelled"],
    ] as const) {
      globalThis.fetch = async () =>
        new Response("admission rejected", { status });
      await assert.rejects(
        consoleClient("test-session").project("p"),
        (error) => {
          assert(error instanceof GeoLedgerError);
          assert.equal(error.code, code);
          assert.equal(error.uncertain, true);
          return true;
        },
      );
    }
  } finally {
    globalThis.fetch = originalFetch;
    if (originalSecret === undefined) delete process.env.GL_WEB_SESSION_SECRET;
    else process.env.GL_WEB_SESSION_SECRET = originalSecret;
  }
});

test("workspace creation and restore return metadata without SDK handle methods", async () => {
  const savedFetch = globalThis.fetch;
  const savedSecret = process.env.GL_WEB_SESSION_SECRET;
  process.env.GL_WEB_SESSION_SECRET = "test-only-session-secret-".repeat(3);
  globalThis.fetch = async () =>
    new Response(
      '{"workspace":"w","base_revision":1,"version":2,"status":"open"}',
    );
  try {
    const client = consoleClient("test-session");
    const results = [
      await client.createWorkspace("p"),
      await client.restore("p", 1n),
    ];
    for (const result of results) {
      assert.deepEqual(Object.keys(result), ["info"]);
      assert.equal(result.info.id, "w");
      assert.equal(result.info.version, 2n);
      // @ts-expect-error HTTP metadata must not promise stateful SDK methods.
      assert.equal(result.save, undefined);
      // @ts-expect-error HTTP metadata must not promise stateful SDK methods.
      assert.equal(result.publish, undefined);
    }
  } finally {
    globalThis.fetch = savedFetch;
    if (savedSecret === undefined) delete process.env.GL_WEB_SESSION_SECRET;
    else process.env.GL_WEB_SESSION_SECRET = savedSecret;
  }
});

test("business transport rejects malformed domain envelopes and preserves exact opaque data", async () => {
  const savedFetch = globalThis.fetch;
  const savedSecret = process.env.GL_WEB_SESSION_SECRET;
  process.env.GL_WEB_SESSION_SECRET = "test-only-session-secret-".repeat(3);
  let response = "{}";
  let status = 200;
  globalThis.fetch = async () => new Response(response, { status });
  const client = consoleClient("test-session");
  const invalid = (error: unknown) =>
    error instanceof GeoLedgerError &&
    error.code === "invalid_response" &&
    error.uncertain;
  const item = {
    cursor: "opaque/%/cursor",
    dataset: "d",
    feature_id: "f",
    fields: ["/properties/a"],
    base: null,
    current: null,
    draft: null,
  };
  const conflicts = {
    head: 2,
    version: 3,
    total: 1,
    next_after: "opaque/%/cursor",
    truncated: false,
    conflicts: [item],
  };
  try {
    for (const coordinate_dimension of [0, 9, "2", null]) {
      response = JSON.stringify({
        dataset: "d",
        name: "D",
        geometry_type: "point",
        coordinate_dimension,
        crs: "EPSG:4326",
      });
      await assert.rejects(client.createDataset("p", "D", "point"), invalid);
    }
    for (const coordinate_dimension of [2, 3]) {
      response = JSON.stringify({
        dataset: "d",
        name: "D",
        geometry_type: "point",
        coordinate_dimension,
        crs: "EPSG:4326",
      });
      assert.equal(
        (await client.createDataset("p", "D", "point")).coordinateDimension,
        coordinate_dimension,
      );
    }
    for (const feature of [
      { type: "Feature", id: "f", properties: 123, geometry: null },
      { type: "Feature", id: "f", properties: {}, geometry: 123 },
      null,
      {},
      { type: "Feature", id: 3, properties: {}, geometry: null },
      { type: "Feature", id: "f", properties: [], geometry: null },
      { type: "Feature", id: "f", properties: {} },
    ]) {
      response = JSON.stringify({
        type: "FeatureCollection",
        crs: "EPSG:4326",
        revision: 1,
        features: [feature],
      });
      await assert.rejects(client.features("p", "d"), invalid);
    }
    for (const malformed of [
      { ...conflicts, head: "2" },
      { ...conflicts, truncated: "false" },
      { ...conflicts, next_after: 3 },
      { ...conflicts, conflicts: [{ ...item, fields: [1] }] },
      { ...conflicts, conflicts: [{ ...item, current: {} }] },
    ]) {
      for (status of [200, 409]) {
        response = JSON.stringify(malformed);
        await assert.rejects(client.conflicts("p", "w"), invalid);
      }
    }
    status = 200;
    response = JSON.stringify(conflicts);
    const result = await client.conflicts("p", "w");
    assert.equal(result.nextAfter, "opaque/%/cursor");
    assert.equal(result.conflicts[0]?.reason, "");
    response =
      '{"type":"FeatureCollection","crs":"EPSG:4326","revision":9007199254740993,"next_after":"opaque/%/cursor","features":[{"type":"Feature","id":"f","properties":{"exact":18446744073709551615,"decimal":1.00000000000000001e18,"snake_case":1},"geometry":null}]}';
    const page = await client.features("p", "d");
    assert.equal(page.revision, 9007199254740993n);
    assert.equal(page.nextAfter, "opaque/%/cursor");
    assert.match(stringifyJson(page.features), /18446744073709551615/);
    assert.match(stringifyJson(page.features), /1\.00000000000000001e18/);
    assert.match(stringifyJson(page.features), /snake_case/);
  } finally {
    globalThis.fetch = savedFetch;
    if (savedSecret === undefined) delete process.env.GL_WEB_SESSION_SECRET;
    else process.env.GL_WEB_SESSION_SECRET = savedSecret;
  }
});

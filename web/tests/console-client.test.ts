import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { consoleClient, authRequest } from "../src/lib/console-client";
import { execute, encode } from "../src/lib/operations";
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
      '{"type":"FeatureCollection","crs":"EPSG:4326","revision":2,"workspace_version":null,"next_after":null,"features":[{"type":"Feature","id":"f","properties":{"snake_case":18446744073709551615,"exact":1.00000000000000001e18,"__proto__":{"safe":1}},"geometry":null}]}';
    const features = await execute(client, {
      action: "features",
      project: "p",
      dataset: "d",
    });
    const raw = stringifyJson(features);
    assert.match(raw, /"snake_case":18446744073709551615/);
    assert.match(raw, /"exact":1.00000000000000001e18/);
    assert.match(raw, /"__proto__":/);
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
            '{"type":"Feature","id":"f","properties":{"exact":1.00000000000000001e18},"geometry":null}',
        },
      ],
    });
    assert.match(requests.at(-1)!.body, /"feature_id":"f"/);
    assert.match(requests.at(-1)!.body, /"exact":1.00000000000000001e18/);
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

test("Session cookie has no credential store, logout revokes database authority and password change rotates", () => {
  const session = readFileSync(
    new URL("../src/lib/session.ts", import.meta.url),
    "utf8",
  );
  assert.match(session, /getIronSession<\{\s*sid\?: string\s*\}>/);
  assert.match(session, /authRequest\("logout"/);
  assert.match(session, /httpOnly:\s*true/);
  assert.match(session, /sameSite:\s*"strict"/);
  assert.match(session, /secure:\s*config.secure/);
  assert.doesNotMatch(
    session,
    /new Map|SessionStore|entry\.token|api_key|password:.*input/,
  );
  const route = readFileSync(
    new URL("../src/app/api/session/route.ts", import.meta.url),
    "utf8",
  );
  assert.match(route, /guard\(request\)/);
  assert.match(route, /login\(result.session\)/);
  assert.match(route, /reply\(\{\s*user:\s*result.user\s*\}\)/);
});

test("commit pagination cannot overwrite the normalized exact revision", async () => {
  const originalFetch = globalThis.fetch;
  const originalSecret = process.env.GL_WEB_SESSION_SECRET;
  process.env.GL_WEB_SESSION_SECRET = "test-only-session-secret-".repeat(3);
  let body = "";
  globalThis.fetch = async (_input, init) => {
    body = String(init?.body);
    return new Response('{"revision":9007199254740993,"changes":[]}');
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

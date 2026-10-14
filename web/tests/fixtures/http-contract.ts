import test from "node:test";
import assert from "node:assert/strict";
import { consoleClient } from "../../src/lib/console-client";
import { execute } from "../../src/lib/operations";
import { failure, HttpError, reply, body } from "../../src/lib/http";
import { send, ApiError } from "../../src/lib/browser-api";
import {
  publication,
  readPublication,
  releasePublication,
} from "../../src/lib/publication";

test("lost committed response -> admission rejection -> success preserves original intent through BFF and browser", async () => {
  const originalFetch = globalThis.fetch;
  const originalSecret = process.env.GL_WEB_SESSION_SECRET;
  process.env.GL_WEB_SESSION_SECRET = "test-only-session-secret-".repeat(3);
  try {
    for (const status of [429, 413, 408, 504]) {
      for (const source of ["upstream", "local", "fallback"]) {
        const intent = publication(
          "p",
          "w",
          "9007199254740993",
          "original message",
        );
        let stored: string | null = JSON.stringify(intent);
        let attempt = 0;
        let committed: string | undefined;
        const upstream: string[] = [];
        globalThis.fetch = async (url, init) => {
          if (url === "/api/console") {
            attempt++;
            if (attempt === 2 && source === "fallback")
              return new Response('{"error":{"message":"rejected"}}', {
                status,
              });
            try {
              if (attempt === 2 && source === "local")
                throw new HttpError(status, "rejected");
              return reply(
                await execute(
                  consoleClient("fixture-session"),
                  JSON.parse(String(init?.body)),
                ),
              );
            } catch (error) {
              return failure(error);
            }
          }
          const body = String(init?.body);
          upstream.push(body);
          if (attempt === 1) {
            committed = body; // Service commits once; delivery of the response fails.
            return new Response(
              new ReadableStream({
                start(controller) {
                  controller.error(new Error("lost response"));
                },
              }),
            );
          }
          if (attempt === 2)
            return new Response('{"error":"admission rejected"}', { status });
          assert.equal(body, committed);
          return new Response('{"revision":2,"version":1,"changes":1}');
        };
        for (let retry = 0; retry < 2; retry++) {
          const pending = readPublication(stored)!;
          await assert.rejects(
            send("/api/console", "POST", pending),
            (error) => {
              assert(error instanceof ApiError);
              if (releasePublication(error.status, error.uncertain, true))
                stored = null;
              return true;
            },
          );
          assert.deepEqual(readPublication(stored), intent);
        }
        assert.deepEqual(
          await send("/api/console", "POST", readPublication(stored)),
          { revision: "2", version: "1", changes: "1" },
        );
        for (const raw of upstream) {
          assert.equal(raw, committed);
          assert.match(raw, /"expected_workspace_version":9007199254740993/);
          assert.equal(JSON.parse(raw).request_id, intent.requestId);
          assert.equal(JSON.parse(raw).message, intent.message);
        }
      }
    }
    for (const status of [429, 413, 408]) {
      const response = failure(new HttpError(status, "local rejection"));
      const body = await response.json();
      assert.equal(body.error.uncertain, true);
      assert.equal(
        body.error.code,
        status === 408 ? "timeout" : "resource_exhausted",
      );
    }
    assert.equal(releasePublication(409, false, true), true);
  } finally {
    globalThis.fetch = originalFetch;
    if (originalSecret === undefined) delete process.env.GL_WEB_SESSION_SECRET;
    else process.env.GL_WEB_SESSION_SECRET = originalSecret;
  }
});

test("malformed feature input stays a definite client error without upstream calls", async () => {
  const originalFetch = globalThis.fetch;
  let calls = 0;
  globalThis.fetch = async () => {
    calls++;
    return new Response("{}");
  };
  try {
    for (const action of ["save", "resolve", "rebase"]) {
      const request = {
        action,
        project: "p",
        workspace: "w",
        version: "1",
        ...(action === "save" ? {} : { head: "0" }),
        edits: [{ dataset: "d", featureId: "f", feature: "{" }],
      };
      let response: Response | undefined;
      try {
        await execute(consoleClient("test-session"), request);
      } catch (error) {
        response = failure(error);
      }
      assert(response);
      assert.equal(response.status, 400);
      const result = await response.json();
      assert.equal(result.error.code, "invalid_argument");
      assert.equal(result.error.uncertain ?? false, false);
    }
    assert.equal(calls, 0);
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test("non-JSON browser errors retain status, redirect and publication semantics", async () => {
  const originalFetch = globalThis.fetch;
  const originalWindow = Object.getOwnPropertyDescriptor(globalThis, "window");
  const redirects: string[] = [];
  Object.defineProperty(globalThis, "window", {
    configurable: true,
    value: {
      location: { replace: (url: string) => redirects.push(url) },
    },
  });
  try {
    for (const raw of ["<html>gateway error</html>", "", "null"]) {
      for (const status of [401, 409, 429, 413, 408, 504]) {
        redirects.length = 0;
        globalThis.fetch = async () => new Response(raw, { status });
        await assert.rejects(send("/api/console", "POST", {}), (error) => {
          assert(error instanceof ApiError);
          assert.equal(error.status, status);
          assert.equal(
            releasePublication(error.status, error.uncertain, true),
            status === 409,
          );
          return true;
        });
        assert.deepEqual(redirects, status === 401 ? ["/login?expired=1"] : []);
      }
    }
  } finally {
    globalThis.fetch = originalFetch;
    if (originalWindow)
      Object.defineProperty(globalThis, "window", originalWindow);
    else Reflect.deleteProperty(globalThis, "window");
  }
});

test("BFF rejects invalid UTF-8 and preserves valid multibyte chunk boundaries", async () => {
  function request(chunks: Uint8Array[]) {
    return new Request("http://localhost/api/console", {
      method: "POST",
      duplex: "half",
      body: new ReadableStream<Uint8Array>({
        start(controller) {
          for (const chunk of chunks) controller.enqueue(chunk);
          controller.close();
        },
      }),
    } as RequestInit);
  }
  for (const invalid of [[255], [192, 175], [240, 159]]) {
    await assert.rejects(
      body(
        request([
          new TextEncoder().encode('{"action":"createProject","name":"'),
          Uint8Array.from(invalid),
          new TextEncoder().encode('"}'),
        ]),
        4096,
      ),
      (error) => error instanceof HttpError && error.status === 400,
    );
  }
  await assert.rejects(
    body(request([new TextEncoder().encode("\ufeff{}")]), 4096),
    (error) => error instanceof HttpError && error.status === 400,
  );
  const value = { name: "中文😀" };
  const bytes = new TextEncoder().encode(JSON.stringify(value));
  assert.deepEqual(
    await body(
      request(Array.from(bytes, (value) => Uint8Array.of(value))),
      4096,
    ),
    value,
  );
});

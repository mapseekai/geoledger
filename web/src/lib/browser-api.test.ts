import assert from "node:assert/strict";
import test from "node:test";
import { call, send, ApiError } from "./browser-api";

test("caller abort prevents requests and propagates its reason through both API routes", async () => {
  const original = globalThis.fetch;
  const controller = new AbortController();
  const reason = new DOMException("dialog closed", "AbortError");
  let calls = 0;
  globalThis.fetch = async () => {
    calls++;
    throw new Error("unexpected fetch");
  };
  try {
    controller.abort(reason);
    await assert.rejects(
      call({ action: "datasets" }, controller.signal),
      (e) => e === reason,
    );
    assert.equal(calls, 0);
    for (const action of ["datasets", "features"]) {
      const pending = new AbortController();
      globalThis.fetch = (_url, init) =>
        new Promise((_resolve, reject) => {
          assert(init?.signal);
          init.signal.addEventListener(
            "abort",
            () => reject(init.signal?.reason),
            { once: true },
          );
        });
      const result = call(
        { action, project: "p", dataset: "d" },
        pending.signal,
      );
      pending.abort(reason);
      await assert.rejects(result, (e) => e === reason);
    }
  } finally {
    globalThis.fetch = original;
  }
});

test("timeout remains an uncertain transport failure and is combined with caller cancellation", async () => {
  const original = globalThis.fetch;
  const originalTimeout = AbortSignal.timeout;
  const deadline = new AbortController();
  AbortSignal.timeout = (ms) => {
    assert.equal(ms, 45_000);
    return deadline.signal;
  };
  globalThis.fetch = (_url, init) =>
    new Promise((_resolve, reject) => {
      assert(init?.signal);
      init.signal.addEventListener("abort", () => reject(init.signal?.reason), {
        once: true,
      });
    });
  try {
    const caller = new AbortController();
    const request = send("/api/console", "POST", {}, false, caller.signal);
    deadline.abort(new DOMException("timeout", "TimeoutError"));
    caller.abort(); // The first abort wins, even when the caller aborts before catch runs.
    await assert.rejects(
      request,
      (e) => e instanceof ApiError && e.status === 0 && e.uncertain,
    );
  } finally {
    globalThis.fetch = original;
    AbortSignal.timeout = originalTimeout;
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

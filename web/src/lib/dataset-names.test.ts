import assert from "node:assert/strict";
import test from "node:test";
import { loadDatasetNames } from "./dataset-names";

test("cancelled dataset loading does not start a later page even when a response ignores abort", async () => {
  const original = globalThis.fetch;
  const controller = new AbortController();
  const reason = new DOMException("unmounted", "AbortError");
  let calls = 0;
  let complete: (response: Response) => void = () => {
    throw new Error("not started");
  };
  globalThis.fetch = () => {
    calls++;
    return new Promise((resolve) => {
      complete = resolve;
    });
  };
  try {
    const result = loadDatasetNames("p", controller.signal);
    controller.abort(reason);
    complete(
      Response.json(
        Array.from({ length: 100 }, (_, i) => ({
          id: String(i),
          name: String(i),
        })),
      ),
    );
    await assert.rejects(result, (e) => e === reason);
    assert.equal(calls, 1);
  } finally {
    globalThis.fetch = original;
  }
});

test("dataset loading preserves pagination and the twenty-page bound", async () => {
  const original = globalThis.fetch;
  let calls = 0;
  globalThis.fetch = async (_url, init) => {
    const request = JSON.parse(String(init?.body));
    assert.equal(request.project, "p");
    assert.equal(request.limit, 100);
    assert.equal(request.after, calls ? String(calls * 100 - 1) : undefined);
    const offset = calls++ * 100;
    return Response.json(
      Array.from({ length: 100 }, (_, i) => ({
        id: String(offset + i),
        name: `name-${offset + i}`,
      })),
    );
  };
  try {
    const names = await loadDatasetNames("p", new AbortController().signal);
    assert.equal(calls, 20);
    assert.equal(names.size, 2000);
    assert.equal(names.get("1999"), "name-1999");
  } finally {
    globalThis.fetch = original;
  }
});

import assert from "node:assert/strict";
import test from "node:test";

test("worker distinguishes a failed browser read from malformed JSON", async () => {
  const messages: { error?: string; rows?: unknown[] }[] = [];
  const worker = {
    onmessage: undefined as
      ((event: MessageEvent<{ file: File }>) => Promise<void>) | undefined,
    postMessage(message: (typeof messages)[number]) {
      messages.push(message);
    },
  };
  const previous = Object.getOwnPropertyDescriptor(globalThis, "self");
  Object.defineProperty(globalThis, "self", {
    value: worker,
    configurable: true,
  });
  try {
    await import("../src/lib/geojson-import.worker");
    assert.ok(worker.onmessage);
    const unreadable = new File(["nonempty"], "large.geojson");
    Object.defineProperty(unreadable, "text", {
      value: async () => "",
    });
    await worker.onmessage(
      new MessageEvent("message", { data: { file: unreadable } }),
    );
    assert.match(messages.pop()?.error ?? "", /浏览器读取文件失败/);

    const malformed = new File(["not JSON"], "invalid.geojson");
    await worker.onmessage(
      new MessageEvent("message", { data: { file: malformed } }),
    );
    const parseError = messages.pop()?.error;
    assert.ok(parseError);
    assert.doesNotMatch(parseError, /浏览器读取文件失败/);

    const valid = new File(
      [
        JSON.stringify({
          type: "FeatureCollection",
          features: [
            {
              type: "Feature",
              id: "one",
              properties: {},
              geometry: { type: "Point", coordinates: [1, 2] },
            },
          ],
        }),
      ],
      "valid.geojson",
    );
    await worker.onmessage(
      new MessageEvent("message", { data: { file: valid } }),
    );
    const imported = messages.pop();
    assert.equal(imported?.error, undefined);
    assert.equal(imported?.rows?.length, 1);
  } finally {
    if (previous) Object.defineProperty(globalThis, "self", previous);
    else Reflect.deleteProperty(globalThis, "self");
  }
});

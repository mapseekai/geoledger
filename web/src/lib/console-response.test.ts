import assert from "node:assert/strict";
import test from "node:test";
import { validateBusinessResponse } from "./console-response";

test("query pages validate and normalize the server continuation without guessing from length", () => {
  const examples = [
    {
      operation: "diff",
      page: { changes: [{ cursor: "dataset/key/1" }] },
      cursor: "dataset/key/1",
    },
    {
      operation: "commit",
      page: { changes: [{ cursor: "dataset/key/1" }] },
      cursor: "dataset/key/1",
    },
    {
      operation: "history",
      page: { commits: [{ revision: 9007199254740993n }] },
      cursor: 9007199254740993n,
    },
    {
      operation: "audit",
      page: { events: [{ id: 9007199254740993n }] },
      cursor: 9007199254740993n,
    },
  ];
  for (const { operation, page, cursor } of examples) {
    assert.deepEqual(
      validateBusinessResponse(operation, { ...page, nextAfter: cursor }),
      { ...page, nextAfter: cursor },
    );
    assert.deepEqual(
      validateBusinessResponse(operation, { ...page, nextAfter: null }),
      page,
    );
    assert.throws(
      () => validateBusinessResponse(operation, page),
      /Invalid business response/,
    );
    assert.throws(
      () => validateBusinessResponse(operation, { ...page, nextAfter: 2 }),
      /Invalid business response/,
    );
    assert.throws(
      () => validateBusinessResponse(operation, []),
      /Invalid business response/,
    );
  }
});

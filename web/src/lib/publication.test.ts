import assert from "node:assert/strict";
import test from "node:test";
import { releasePublication } from "./publication";

test("pending publication survives expired authentication but releases definitive merge conflicts", () => {
  for (const status of [401, 403, 404, 408, 413, 429, 504])
    assert.equal(releasePublication(status, false, true), false);
  assert.equal(releasePublication(409, false, true), true);
  assert.equal(releasePublication(400, false, true), true);
  assert.equal(releasePublication(503, true, true), false);
  assert.equal(releasePublication(401, false, false), true);
});

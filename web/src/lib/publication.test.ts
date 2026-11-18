import assert from "node:assert/strict";
import test from "node:test";
import { releasePublication } from "./publication";

test("pending publication survives expired authentication but releases definitive merge conflicts", () => {
  for (const status of [400, 401, 403, 404, 408, 413, 429, 504])
    assert.equal(releasePublication(status, false, true), false);
  assert.equal(releasePublication(409, false, true), true);
  assert.equal(releasePublication(400, false, false), true);
  assert.equal(releasePublication(503, true, true), false);
  assert.equal(releasePublication(401, false, false), true);
});

test("source rejection releases only a first attempt, while absent details stay conservative", () => {
  const code = "publication_attempt_rejected";
  for (const uncertain of [false, true]) {
    assert.equal(releasePublication(503, uncertain, false, code), true);
    assert.equal(releasePublication(503, uncertain, true, code), false);
  }
  assert.equal(releasePublication(503, true, false), false);
  assert.equal(releasePublication(503, true, true, "unavailable"), false);
  assert.equal(releasePublication(422, false, true, "invalid_argument"), false);
});

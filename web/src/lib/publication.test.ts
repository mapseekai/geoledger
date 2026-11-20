import assert from "node:assert/strict";
import test from "node:test";
import { releasePublication } from "./publication";

test("an earlier unknown publication only releases on a permanent workspace version proof", () => {
  for (const status of [400, 401, 403, 404, 408, 409, 413, 422, 429, 504])
    assert.equal(releasePublication(status, false, true), false);
  for (const code of [
    "conflict",
    "publication_attempt_rejected",
    "invalid_argument",
    "unknown_business_error",
  ])
    assert.equal(releasePublication(409, false, true, code), false);
  assert.equal(
    releasePublication(409, false, true, "workspace_version_conflict"),
    true,
  );
  assert.equal(
    releasePublication(409, true, true, "workspace_version_conflict"),
    false,
  );
  assert.equal(
    releasePublication(503, false, true, "workspace_version_conflict"),
    false,
  );
});

test("a first definitive rejection releases while transport uncertainty keeps the original intent", () => {
  for (const code of [
    "conflict",
    "publication_attempt_rejected",
    "invalid_argument",
    "workspace_version_conflict",
  ])
    assert.equal(releasePublication(400, false, false, code), true);
  assert.equal(releasePublication(401, false, false), true);
  assert.equal(releasePublication(503, true, false), false);
  assert.equal(
    releasePublication(400, true, false, "publication_attempt_rejected"),
    false,
  );
});

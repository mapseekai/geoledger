import assert from "node:assert/strict";
import test from "node:test";
import { keyExpiryInput, keyExpiryChange } from "../src/lib/api-key-expiry";

test("name-only edits omit expiry and retain exact seconds, milliseconds and timezone", () => {
  for (const expiry of [
    null,
    "2030-01-01T12:00:45Z",
    "2030-01-01T12:00:45.123+08:00",
  ]) {
    assert.equal(keyExpiryChange(expiry, keyExpiryInput(expiry)), undefined);
    assert.equal(
      JSON.stringify({
        name: "renamed",
        expires_at: keyExpiryChange(expiry, keyExpiryInput(expiry)),
      }),
      '{"name":"renamed"}',
    );
  }
  assert.equal(
    keyExpiryInput("2030-01-01T12:00:45+08:00"),
    "2030-01-01T04:00:45",
  );
});
test("explicit expiry changes use UTC and clearing removes the expiry", () => {
  assert.equal(keyExpiryChange(undefined, ""), null);
  assert.equal(keyExpiryChange("2030-01-01T12:00:45Z", ""), null);
  assert.equal(
    keyExpiryChange(null, "2030-01-01T12:00:46"),
    "2030-01-01T12:00:46.000Z",
  );
  assert.throws(() => keyExpiryChange(null, "invalid"));
});

import assert from "node:assert/strict";
import test from "node:test";
import { SessionStore } from "../src/lib/session-store";

test("revoked, idle and expired console sessions stop resolving", () => {
  const store = new SessionStore({
    absoluteMs: 1000,
    idleMs: 100,
    maxSessions: 2,
  });
  const a = store.create("token-a", 0);
  assert.equal(store.get(a, 50)?.token, "token-a");
  assert.equal(store.get(a, 140)?.token, "token-a");
  assert.equal(store.get(a, 300), undefined, "idle timeout");
  const b = store.create("token-b", 0);
  for (let t = 90; t <= 990; t += 90)
    assert.equal(store.get(b, t)?.token, "token-b");
  assert.equal(store.get(b, 1001), undefined, "absolute lifetime");
  const c = store.create("token-c", 2000);
  store.revoke(c);
  assert.equal(store.get(c, 2001), undefined, "logout revokes server-side");
  store.create("x", 3000);
  store.create("y", 3000);
  store.create("z", 3000);
  assert.equal(store.size, 2, "bounded");
  assert.equal(store.get(undefined), undefined);
});

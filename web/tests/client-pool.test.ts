import assert from "node:assert/strict";
import test from "node:test";
import { ClientPool } from "../src/lib/client-pool";

class Fake {
  closed = 0;
  constructor(readonly token: string) {}
  close() {
    this.closed += 1;
  }
}

test("one client per credential is reused until idle, evicted or displaced", () => {
  const made: Fake[] = [];
  const pool = new ClientPool(
    (token) => {
      const client = new Fake(token);
      made.push(client);
      return client;
    },
    { max: 2, idleMs: 100 },
  );
  const a1 = pool.acquire("token-a", 0);
  a1.release();
  const a2 = pool.acquire("token-a", 50);
  assert.equal(a2.client, a1.client, "reused within the idle window");
  a2.release();
  a2.release(); // double release is harmless
  assert.equal(made.length, 1);

  pool.acquire("token-b", 60).release();
  pool.acquire("token-a", 70).release(); // a is now most recently used
  pool.acquire("token-c", 80).release(); // displaces b, the least recently used
  assert.equal(pool.size, 2);
  assert.equal(made.find((c) => c.token === "token-b")?.closed, 1);
  assert.equal(made.find((c) => c.token === "token-a")?.closed, 0);

  pool.acquire("token-c", 500).release(); // a idle for 430 ms: pruned
  assert.equal(made.find((c) => c.token === "token-a")?.closed, 1);
  assert.equal(pool.size, 1);

  const fresh = pool.acquire("token-a", 510);
  assert.notEqual(
    fresh.client,
    a1.client,
    "a pruned client is never handed out again",
  );
  fresh.release();
});

test("clients serving a request are closed only after their last lease", () => {
  const pool = new ClientPool((token) => new Fake(token), {
    max: 1,
    idleMs: 1000,
  });
  const busy = pool.acquire("token-a", 0);
  const second = pool.acquire("token-a", 1);
  pool.evict("token-a");
  assert.equal(busy.client.closed, 0, "in flight");
  busy.release();
  assert.equal(busy.client.closed, 0, "another request still uses it");
  second.release();
  assert.equal(busy.client.closed, 1);

  const x = pool.acquire("token-x", 10);
  const y = pool.acquire("token-y", 11); // pool full of busy clients: x retires
  assert.equal(pool.size, 1);
  assert.equal(x.client.closed, 0);
  x.release();
  assert.equal(x.client.closed, 1);
  y.release();
  pool.closeAll();
  assert.equal(y.client.closed, 1);
  assert.equal(pool.size, 0);
});

import { createHash } from "node:crypto";

export interface Closeable {
  close(): void;
}
export interface ClientPoolLimits {
  /** Most clients kept open at once; the least recently used idle one is closed first. */
  max: number;
  /** Clients unused for this long are closed on the next acquire. */
  idleMs: number;
}
export interface Lease<C> {
  client: C;
  /** Return the client to the pool. Safe to call more than once. */
  release(): void;
}
interface Entry<C> {
  client: C;
  lastUsed: number;
  leases: number;
  retired: boolean;
}

/**
 * Reuses one gRPC client (one HTTP/2 connection, one TLS handshake) per credential
 * instead of dialing per request. Keys are SHA-256 digests, so the map never holds
 * tokens as keys. Evicted clients that are still serving a request are closed when
 * their last lease is released.
 */
export class ClientPool<C extends Closeable> {
  #entries = new Map<string, Entry<C>>();
  constructor(
    private readonly create: (token: string) => C,
    private readonly limits: ClientPoolLimits,
  ) {}
  static key(token: string): string {
    return createHash("sha256").update(token).digest("base64url");
  }
  acquire(token: string, now = Date.now()): Lease<C> {
    this.prune(now);
    const key = ClientPool.key(token);
    let entry = this.#entries.get(key);
    if (entry) {
      // Re-insert to keep Map iteration order = least recently used first.
      this.#entries.delete(key);
    } else {
      this.#makeRoom();
      entry = {
        client: this.create(token),
        lastUsed: now,
        leases: 0,
        retired: false,
      };
    }
    entry.lastUsed = now;
    entry.leases += 1;
    this.#entries.set(key, entry);
    const leased = entry;
    let released = false;
    return {
      client: leased.client,
      release: () => {
        if (released) return;
        released = true;
        leased.leases -= 1;
        if (leased.retired && leased.leases === 0) leased.client.close();
      },
    };
  }
  /** Close the client for a credential, e.g. after logout or an authentication failure. */
  evict(token: string): void {
    const key = ClientPool.key(token);
    const entry = this.#entries.get(key);
    if (entry) this.#retire(key, entry);
  }
  prune(now = Date.now()): void {
    for (const [key, entry] of this.#entries)
      if (entry.leases === 0 && now - entry.lastUsed > this.limits.idleMs)
        this.#retire(key, entry);
  }
  closeAll(): void {
    for (const [key, entry] of this.#entries) this.#retire(key, entry);
  }
  get size(): number {
    return this.#entries.size;
  }
  #makeRoom(): void {
    if (this.#entries.size < this.limits.max) return;
    for (const [key, entry] of this.#entries) {
      if (this.#entries.size < this.limits.max) return;
      if (entry.leases === 0) this.#retire(key, entry);
    }
    // Every client is busy: retire the least recently used anyway; it closes
    // once its in-flight requests finish.
    for (const [key, entry] of this.#entries) {
      if (this.#entries.size < this.limits.max) return;
      this.#retire(key, entry);
    }
  }
  #retire(key: string, entry: Entry<C>): void {
    this.#entries.delete(key);
    entry.retired = true;
    if (entry.leases === 0) entry.client.close();
  }
}

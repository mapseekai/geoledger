import { randomBytes } from "node:crypto";

/**
 * Server-side console sessions. The browser cookie only carries an opaque, encrypted
 * session ID; the GeoLedger credential stays in server memory. Logout deletes the entry,
 * so a copied cookie stops working immediately. A restart signs everyone out; run one
 * console instance (or sticky sessions) per session store.
 */
export interface SessionEntry {
  token: string;
  createdAt: number;
  lastSeen: number;
}
export interface SessionLimits {
  absoluteMs: number;
  idleMs: number;
  maxSessions: number;
}
export class SessionStore {
  #entries = new Map<string, SessionEntry>();
  constructor(private readonly limits: SessionLimits) {}
  create(token: string, now = Date.now()): string {
    this.prune(now);
    while (this.#entries.size >= this.limits.maxSessions) {
      // Map iteration order is insertion order: evict the oldest session.
      const oldest = this.#entries.keys().next().value;
      if (oldest === undefined) break;
      this.#entries.delete(oldest);
    }
    const id = randomBytes(32).toString("base64url");
    this.#entries.set(id, { token, createdAt: now, lastSeen: now });
    return id;
  }
  get(id: string | undefined, now = Date.now()): SessionEntry | undefined {
    if (!id) return undefined;
    const entry = this.#entries.get(id);
    if (!entry) return undefined;
    if (
      now - entry.createdAt > this.limits.absoluteMs ||
      now - entry.lastSeen > this.limits.idleMs
    ) {
      this.#entries.delete(id);
      return undefined;
    }
    entry.lastSeen = now;
    return entry;
  }
  revoke(id: string | undefined): void {
    if (id) this.#entries.delete(id);
  }
  prune(now = Date.now()): void {
    for (const [id, entry] of this.#entries) {
      if (
        now - entry.createdAt > this.limits.absoluteMs ||
        now - entry.lastSeen > this.limits.idleMs
      )
        this.#entries.delete(id);
    }
  }
  get size(): number {
    return this.#entries.size;
  }
}

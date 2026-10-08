import "server-only";
import { cookies } from "next/headers";
import { getIronSession } from "iron-session";
import { configuration } from "./config";
import { SessionStore } from "./session-store";

const ABSOLUTE_SECONDS = 8 * 60 * 60;
function idleMinutes() {
  const value = Number(process.env.GL_WEB_SESSION_IDLE_MINUTES ?? 60);
  return Number.isFinite(value) && value >= 1 ? Math.min(value, 8 * 60) : 60;
}
// Survive module reloads in development; production runs one module instance.
const holder = globalThis as typeof globalThis & {
  __geoledgerSessions?: SessionStore;
};
function store() {
  holder.__geoledgerSessions ??= new SessionStore({
    absoluteMs: ABSOLUTE_SECONDS * 1000,
    idleMs: idleMinutes() * 60 * 1000,
    maxSessions: 10_000,
  });
  return holder.__geoledgerSessions;
}
/** Current console session. `token` is resolved from the server-side store. */
export async function session() {
  const config = configuration();
  const cookie = await getIronSession<{ sid?: string }>(await cookies(), {
    password: config.password,
    cookieName: "gl_session",
    ttl: ABSOLUTE_SECONDS,
    cookieOptions: {
      httpOnly: true,
      secure: config.secure,
      sameSite: "strict",
      path: "/",
    },
  });
  const entry = store().get(cookie.sid);
  return {
    token: entry?.token,
    /** Start a new server-side session; any previous one for this cookie is revoked. */
    async login(token: string) {
      store().revoke(cookie.sid);
      cookie.sid = store().create(token);
      await cookie.save();
    },
    /** Revoke the server-side session and clear the cookie. */
    destroy() {
      store().revoke(cookie.sid);
      cookie.destroy();
    },
  };
}

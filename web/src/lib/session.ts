import "server-only";
import { GeoLedgerError } from "@geoledger/client";
import { cookies } from "next/headers";
import { getIronSession } from "iron-session";
import { configuration } from "./config";
import { authRequest } from "./console-client";

/** The database is the only Session authority; the cookie holds no password or API Key. */
export async function session() {
  const config = configuration();
  const jar = await cookies();
  if (jar.getAll("gl_session").length > 1)
    throw new GeoLedgerError("unauthenticated", "会话凭证无效。");
  const cookie = await getIronSession<{ sid?: string }>(jar, {
    password: config.password,
    cookieName: "gl_session",
    ttl: 8 * 60 * 60,
    cookieOptions: {
      httpOnly: true,
      secure: config.secure,
      sameSite: "strict",
      path: "/",
    },
  });
  return {
    sid: cookie.sid,
    async login(sid: string) {
      if (cookie.sid && cookie.sid !== sid) {
        try {
          await authRequest("logout", {}, cookie.sid);
        } catch (error) {
          if (!(
            error instanceof Error &&
            "code" in error &&
            error.code === "unauthenticated"
          ))
            throw error;
        }
      }
      cookie.sid = sid;
      await cookie.save();
    },
    async destroy() {
      if (cookie.sid) {
        try {
          await authRequest("logout", {}, cookie.sid);
        } catch (error) {
          if (!(
            error instanceof Error &&
            "code" in error &&
            error.code === "unauthenticated"
          ))
            throw error;
        }
      }
      cookie.destroy();
    },
    clear() {
      cookie.destroy();
    },
  };
}

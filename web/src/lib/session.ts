import "server-only";
import { cookies } from "next/headers";
import { getIronSession } from "iron-session";
import { configuration } from "./config";
export async function session() {
  const config = configuration();
  return getIronSession<{ token?: string }>(await cookies(), {
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
}

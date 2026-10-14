import { z } from "zod";
import { session } from "@/lib/session";
import { authRequest } from "@/lib/console-client";
import type { AuthResult } from "@/lib/auth-model";
import { body, failure, guard, HttpError, reply } from "@/lib/http";
export const runtime = "nodejs";
const operation = z.enum([
  "me",
  "change_password",
  "create_user",
  "list_users",
  "update_user",
  "reset_password",
  "create_api_key",
  "list_api_keys",
  "update_api_key",
  "revoke_api_key",
  "delete_api_key",
]);
export async function POST(request: Request) {
  try {
    guard(request);
    const auth = await session();
    if (!auth.sid) throw new HttpError(401, "请登录控制台。");
    const { action, ...input } = z
      .object({ action: operation })
      .passthrough()
      .parse(await body(request, 16384));
    const result = await authRequest(action, input, auth.sid);
    if (action === "change_password") {
      const changed = result as AuthResult;
      // Password change already revokes the previous database Session.
      auth.clear();
      await (await session()).login(changed.session);
      return reply({ user: changed.user });
    }
    return reply(result);
  } catch (error) {
    return failure(error);
  }
}

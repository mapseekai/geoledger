import { authRequestSchema } from "@/lib/auth-requests";
import { session } from "@/lib/session";
import { authRequest } from "@/lib/console-client";
import type { AuthResult } from "@/lib/auth-model";
import { body, failure, guard, HttpError, reply } from "@/lib/http";
export const runtime = "nodejs";
export async function POST(request: Request) {
  try {
    guard(request);
    const auth = await session();
    if (!auth.sid) throw new HttpError(401, "请登录控制台。");
    const { action, ...input } = authRequestSchema.parse(
      await body(request, 16384),
    );
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

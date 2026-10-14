import { passwordError } from "@/lib/password";
import { z } from "zod";
import { session } from "@/lib/session";
import { authRequest } from "@/lib/console-client";
import type { AuthResult } from "@/lib/auth-model";
import { body, failure, guard, reply } from "@/lib/http";
export const runtime = "nodejs";
const login = z.union([
  z.strictObject({
    username: z.string().min(1).max(128),
    password: z.string().refine((value) => !passwordError(value, 1)),
  }),
  z.strictObject({ api_key: z.string().regex(/^glk_[a-f0-9]{64}$/) }),
]);
export async function POST(request: Request) {
  try {
    guard(request);
    const input = login.parse(await body(request, 8192));
    const result = await authRequest<AuthResult>(
      "api_key" in input ? "login_api_key" : "login_password",
      input,
    );
    await (await session()).login(result.session);
    return reply({ user: result.user });
  } catch (error) {
    return failure(error);
  }
}
export async function DELETE(request: Request) {
  try {
    guard(request);
    await (await session()).destroy();
    return reply({ ok: true });
  } catch (error) {
    return failure(error);
  }
}

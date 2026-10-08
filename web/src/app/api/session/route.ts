import { Client } from "@geoledger/client";
import { z } from "zod";
import { session } from "@/lib/session";
import { configuration } from "@/lib/config";
import { body, failure, guard, reply } from "@/lib/http";
export const runtime = "nodejs";
export async function POST(request: Request) {
  let client: Client | undefined;
  try {
    guard(request);
    const { token } = z
      .strictObject({
        token: z
          .string()
          .min(1)
          .max(2000)
          .regex(/^[\x21-\x7e]+$/),
      })
      .parse(await body(request, 4096));
    client = new Client(configuration().endpoint, token);
    const info = await client.info();
    const auth = await session();
    auth.token = token;
    await auth.save();
    return reply({ info });
  } catch (error) {
    return failure(error);
  } finally {
    client?.close();
  }
}
export async function DELETE(request: Request) {
  try {
    guard(request);
    (await session()).destroy();
    return reply({ ok: true });
  } catch (error) {
    return failure(error);
  }
}

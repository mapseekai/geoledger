import { Client, GeoLedgerError } from "@geoledger/client";
import { session } from "@/lib/session";
import { configuration } from "@/lib/config";
import { execute } from "@/lib/operations";
import { body, failure, guard, HttpError, reply } from "@/lib/http";
export const runtime = "nodejs";
export async function POST(request: Request) {
  let client: Client | undefined;
  try {
    guard(request);
    const auth = await session();
    if (!auth.token) throw new HttpError(401, "请登录控制台。");
    client = new Client(configuration().endpoint, auth.token);
    try {
      return reply(await execute(client, await body(request)));
    } catch (error) {
      if (error instanceof GeoLedgerError && error.code === "unauthenticated")
        auth.destroy();
      throw error;
    }
  } catch (error) {
    return failure(error);
  } finally {
    client?.close();
  }
}

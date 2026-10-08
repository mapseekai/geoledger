import { GeoLedgerError } from "@geoledger/client";
import { session } from "@/lib/session";
import { clients } from "@/lib/clients";
import { execute } from "@/lib/operations";
import { body, failure, guard, HttpError, reply } from "@/lib/http";
export const runtime = "nodejs";
export async function POST(request: Request) {
  try {
    guard(request);
    const auth = await session();
    const token = auth.token;
    if (!token) throw new HttpError(401, "请登录控制台。");
    const lease = clients().acquire(token);
    try {
      return reply(await execute(lease.client, await body(request)));
    } catch (error) {
      if (error instanceof GeoLedgerError && error.code === "unauthenticated") {
        auth.destroy();
        clients().evict(token);
      }
      throw error;
    } finally {
      lease.release();
    }
  } catch (error) {
    return failure(error);
  }
}

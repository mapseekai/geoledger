import { GeoLedgerError } from "@geoledger/client";
import { session } from "@/lib/session";
import { clients } from "@/lib/clients";
import { execute } from "@/lib/operations";
import { body, failure, guard, HttpError, reply, geoReply } from "@/lib/http";
export const runtime = "nodejs";
export async function POST(request: Request) {
  try {
    guard(request);
    const auth = await session();
    const token = auth.token;
    if (!token) throw new HttpError(401, "请登录控制台。");
    const lease = clients().acquire(token);
    try {
      const data = await execute(lease.client, await body(request));
      return data &&
        typeof data === "object" &&
        "type" in data &&
        data.type === "FeatureCollection"
        ? geoReply(data)
        : reply(data);
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

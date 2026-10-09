import { GeoLedgerError } from "@geoledger/client";
import { session } from "@/lib/session";
import { clients } from "@/lib/clients";
import { featureCollection, featureQuery } from "@/lib/feature-collection";
import { failure, geoReply, guardRead, HttpError } from "@/lib/http";
export const runtime = "nodejs";
export async function GET(
  request: Request,
  context: { params: Promise<{ project: string; dataset: string }> },
) {
  try {
    guardRead(request);
    const auth = await session(),
      token = auth.token;
    if (!token) throw new HttpError(401, "请登录控制台。");
    const { project, dataset } = await context.params;
    const query = featureQuery(
      project,
      dataset,
      new URL(request.url).searchParams,
    );
    const lease = clients().acquire(token);
    try {
      return geoReply(await featureCollection(lease.client, query));
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

import { session } from "@/lib/session";
import { consoleClient } from "@/lib/console-client";
import { featureCollection, featureQuery } from "@/lib/feature-collection";
import { failure, geoReply, guardRead, HttpError } from "@/lib/http";
export const runtime = "nodejs";
export async function GET(
  request: Request,
  context: { params: Promise<{ project: string; dataset: string }> },
) {
  try {
    guardRead(request);
    const auth = await session();
    if (!auth.sid) throw new HttpError(401, "请登录控制台。");
    const { project, dataset } = await context.params;
    const query = featureQuery(
      project,
      dataset,
      new URL(request.url).searchParams,
    );
    return geoReply(await featureCollection(consoleClient(auth.sid), query));
  } catch (error) {
    return failure(error);
  }
}

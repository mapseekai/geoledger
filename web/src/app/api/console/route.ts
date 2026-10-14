import { session } from "@/lib/session";
import { consoleClient } from "@/lib/console-client";
import { execute } from "@/lib/operations";
import { body, failure, guard, HttpError, reply, geoReply } from "@/lib/http";
export const runtime = "nodejs";
export async function POST(request: Request) {
  try {
    guard(request);
    const auth = await session();
    if (!auth.sid) throw new HttpError(401, "请登录控制台。");
    const data = await execute(consoleClient(auth.sid), await body(request));
    return data &&
      typeof data === "object" &&
      "type" in data &&
      data.type === "FeatureCollection"
      ? geoReply(data)
      : reply(data);
  } catch (error) {
    return failure(error);
  }
}

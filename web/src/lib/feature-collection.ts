import { GeoLedgerError, type Client } from "@geoledger/client";
import { requestSchema } from "./requests";
import { featureItemsUrl } from "./feature-links";
export async function featureCollection(client: Client, input: unknown) {
  const r = requestSchema.parse(input);
  if (r.action !== "features") throw new Error("Expected features query");
  const page = await client.features(r.project, r.dataset, {
    ...r,
    revision: r.revision === undefined ? undefined : BigInt(r.revision),
  });
  if (
    r.workspaceVersion !== undefined &&
    page.workspaceVersion !== BigInt(r.workspaceVersion)
  )
    throw new GeoLedgerError("conflict", "工作区已变化，请刷新后重新加载。");
  const snapshot = {
    ...r,
    revision: r.workspace ? undefined : page.revision.toString(),
    workspaceVersion: page.workspaceVersion?.toString(),
  };
  const links = [
    {
      rel: "self",
      type: "application/geo+json",
      href: featureItemsUrl(snapshot),
    },
  ];
  if (page.nextAfter)
    links.push({
      rel: "next",
      type: "application/geo+json",
      href: featureItemsUrl({ ...snapshot, after: page.nextAfter }),
    });
  return {
    type: "FeatureCollection" as const,
    features: page.features,
    links,
    numberReturned: page.features.length,
    timeStamp: new Date().toISOString(),
    revision: page.revision.toString(),
    workspaceVersion: page.workspaceVersion?.toString(),
    nextAfter: page.nextAfter,
  };
}
export function featureQuery(
  project: string,
  dataset: string,
  params: URLSearchParams,
) {
  const input: Record<string, unknown> = {
    action: "features",
    project,
    dataset,
  };
  for (const [key, value] of params) {
    if (key in input || params.getAll(key).length !== 1)
      throw new GeoLedgerError("invalid_argument", "重复或不支持的查询参数。");
    input[key] =
      key === "limit"
        ? Number(value)
        : key === "bbox"
          ? value.split(",").map(Number)
          : value;
  }
  return requestSchema.parse(input);
}

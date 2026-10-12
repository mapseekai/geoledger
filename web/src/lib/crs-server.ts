import definitions from "epsg-index/all.json";
import type { Dataset } from "@geoledger/client";

/** Resolve locally so the browser downloads only its dataset's definition. */
export function datasetCrs(dataset: Dataset) {
  const code = /^EPSG:(\d+)$/i.exec(dataset.crs)?.[1];
  const entry = code
    ? (definitions as Record<string, { wkt: string | null }>)[code]
    : undefined;
  return { ...dataset, crsDefinition: entry?.wkt ?? dataset.crs };
}

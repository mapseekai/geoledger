import type { Conflict, Conflicts, Feature } from "@geoledger/client";
import { GeoLedgerError } from "@geoledger/client";
import { isLosslessNumber } from "lossless-json";

function invalid(): never {
  throw new GeoLedgerError(
    "invalid_response",
    "Invalid business response",
    undefined,
    undefined,
    true,
  );
}
function object(value: unknown): Record<string, unknown> {
  if (
    !value ||
    typeof value !== "object" ||
    Array.isArray(value) ||
    isLosslessNumber(value)
  )
    return invalid();
  return value as Record<string, unknown>;
}
function text(value: unknown): string {
  return typeof value === "string" ? value : invalid();
}
function integer(value: unknown): bigint {
  return typeof value === "bigint" && value >= 0n ? value : invalid();
}
/** Check the envelope without converting lossless property or geometry values. */
function feature(value: unknown): Feature {
  const row = object(value);
  if (row.type !== "Feature" || typeof row.id !== "string") return invalid();
  object(row.properties);
  if (row.geometry !== null) object(row.geometry);
  // JSON parsing already established JSON-valued children. Keep their lossless tokens intact.
  return {
    ...row,
    type: "Feature",
    id: row.id,
    properties: row.properties,
    geometry: row.geometry,
  } as Feature;
}
function nullableFeature(value: unknown): Feature | null {
  return value === null ? null : feature(value);
}
function conflict(value: unknown): Conflict {
  const row = object(value);
  if (
    !Array.isArray(row.fields) ||
    !row.fields.every((field) => typeof field === "string")
  )
    return invalid();
  return {
    cursor: text(row.cursor),
    dataset: text(row.dataset),
    featureId: text(row.featureId),
    fields: row.fields,
    base: nullableFeature(row.base),
    current: nullableFeature(row.current),
    draft: nullableFeature(row.draft),
    // The HTTP protocol omits this field for ordinary merge conflicts.
    reason: row.reason === undefined ? "" : text(row.reason),
    ...(row.resolvedAgainstRevision == null
      ? {}
      : { resolvedAgainstRevision: integer(row.resolvedAgainstRevision) }),
  };
}
export function decodeConflicts(value: unknown): Conflicts {
  const row = object(value);
  if (!Array.isArray(row.conflicts) || typeof row.truncated !== "boolean")
    return invalid();
  return {
    head: integer(row.head),
    version: integer(row.version),
    total: integer(row.total),
    truncated: row.truncated,
    conflicts: row.conflicts.map(conflict),
    ...(row.nextAfter == null ? {} : { nextAfter: text(row.nextAfter) }),
  };
}
function dataset(value: unknown): unknown {
  const row = object(value);
  if (
    row.geometryType !== "point" &&
    row.geometryType !== "line" &&
    row.geometryType !== "polygon"
  )
    return invalid();
  if (row.coordinateDimension !== 2 && row.coordinateDimension !== 3)
    return invalid();
  text(row.dataset);
  text(row.name);
  text(row.crs);
  return value;
}
/** Validate the domain envelopes that otherwise depend on transport assertions. */
export function validateBusinessResponse(
  operation: string,
  value: unknown,
): unknown {
  if (operation === "conflicts") return decodeConflicts(value);
  if (operation === "list_datasets") {
    if (!Array.isArray(value)) return invalid();
    value.forEach(dataset);
  } else if (["create_dataset", "rename_dataset"].includes(operation))
    dataset(value);
  else if (operation === "features") {
    const row = object(value);
    text(row.crs);
    integer(row.revision);
    if (row.workspaceVersion != null) integer(row.workspaceVersion);
    if (row.nextAfter != null) text(row.nextAfter);
    if (row.type === "Feature") feature(row);
    else {
      if (row.type !== "FeatureCollection" || !Array.isArray(row.features))
        return invalid();
      row.features.forEach(feature);
    }
  }
  return value;
}

import { stringify } from "lossless-json";
import { parseLosslessJson } from "./conflicts";

type JsonObject = Record<string, unknown>;
export type ChangeFeature = JsonObject & {
  properties?: JsonObject | null;
  geometry?: unknown;
};
export type ChangeKind = "added" | "modified" | "deleted";
export type ParsedChange = {
  cursor: string;
  dataset: string;
  featureId: string;
  kind: ChangeKind;
  before: ChangeFeature | null;
  after: ChangeFeature | null;
};
/** Workspace diffs carry base/draft; published commits carry before/after. */
export function parseChange(cursor: string, raw: string): ParsedChange {
  const value = parseLosslessJson(raw) as JsonObject;
  const pick = (a: string, b: string) =>
    ((value[a] ?? value[b] ?? null) as ChangeFeature | null) || null;
  const before =
    "before" in value || "after" in value
      ? pick("before", "base")
      : pick("base", "before");
  const after =
    "before" in value || "after" in value
      ? pick("after", "draft")
      : pick("draft", "after");
  return {
    cursor,
    dataset: String(value.dataset ?? ""),
    featureId: String(value.featureId ?? value.feature_id ?? ""),
    kind: !before ? "added" : !after ? "deleted" : "modified",
    before,
    after,
  };
}
export type DiffRow = {
  key: string;
  label: string;
  before?: unknown;
  after?: unknown;
  hasBefore: boolean;
  hasAfter: boolean;
  changed: boolean;
  geometry: boolean;
};
function props(feature: ChangeFeature | null): JsonObject {
  return feature?.properties && typeof feature.properties === "object"
    ? feature.properties
    : {};
}
export function diffRows(change: ParsedChange): DiffRow[] {
  const keys: string[] = [];
  for (const source of [props(change.before), props(change.after)])
    for (const key of Object.keys(source))
      if (!keys.includes(key)) keys.push(key);
  const same = (a: unknown, b: unknown) => stringify(a) === stringify(b);
  const geometry: DiffRow = {
    key: "\0geometry",
    label: "几何",
    before: change.before?.geometry,
    after: change.after?.geometry,
    hasBefore: !!change.before,
    hasAfter: !!change.after,
    changed:
      !change.before ||
      !change.after ||
      !same(change.before.geometry, change.after.geometry),
    geometry: true,
  };
  return [
    geometry,
    ...keys.map((key) => {
      const b = props(change.before),
        a = props(change.after);
      const hasBefore = Object.hasOwn(b, key),
        hasAfter = Object.hasOwn(a, key);
      return {
        key,
        label: key,
        before: b[key],
        after: a[key],
        hasBefore,
        hasAfter,
        changed: hasBefore !== hasAfter || !same(b[key], a[key]),
        geometry: false,
      };
    }),
  ];
}
/** A readable feature label: the name-like property when present. */
export function featureName(
  feature: { properties?: JsonObject | null } | null | undefined,
): string | undefined {
  const p = feature?.properties;
  if (!p || typeof p !== "object") return;
  for (const key of [
    "name",
    "名称",
    "title",
    "label",
    "NAME",
    "Name",
    "code",
  ]) {
    const value = p[key];
    if (typeof value === "string" && value.trim()) return value;
  }
}

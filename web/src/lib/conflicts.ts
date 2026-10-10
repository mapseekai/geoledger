import { parse, stringify } from "lossless-json";

export type ConflictSide = "base" | "current" | "draft";
type JsonObject = Record<string, unknown>;
export type ConflictFeature = JsonObject & {
  type: "Feature";
  properties?: JsonObject | null;
  geometry?: unknown;
};
export type Conflict = {
  cursor?: string;
  dataset: string;
  featureId: string;
  fields: string[];
  base: ConflictFeature | null;
  current: ConflictFeature | null;
  draft: ConflictFeature | null;
  reason?: string;
  resolvedAgainstRevision?: bigint | string;
};
export type MergedFeature = { feature: ConflictFeature; text: string };

export function parseLosslessJson(raw: string): unknown {
  let marker = "\0geoledger_proto_key";
  for (let number = 0; raw.includes(JSON.stringify(marker)); number++)
    marker = `\0geoledger_proto_key_${number}`;
  const parts: string[] = [];
  let copiedUntil = 0;
  for (let index = 0; index < raw.length;) {
    if (raw[index] !== '"') {
      index++;
      continue;
    }
    const start = index++;
    while (index < raw.length) {
      if (raw[index] === "\\") index += 2;
      else if (raw[index++] === '"') break;
    }
    const token = raw.slice(start, index);
    let after = index;
    while (/\s/.test(raw[after] ?? "")) after++;
    if (raw[after] === ":" && JSON.parse(token) === "__proto__") {
      parts.push(raw.slice(copiedUntil, start), JSON.stringify(marker));
      copiedUntil = index;
    }
  }
  const changed = parts.length > 0;
  if (changed) parts.push(raw.slice(copiedUntil));
  const value = parse(changed ? parts.join("") : raw);
  const restore = (input: unknown): unknown => {
    if (Array.isArray(input)) return input.map(restore);
    if (!input || typeof input !== "object") return input;
    const object = input as Record<string, unknown>;
    for (const key of Object.keys(object)) {
      const child = restore(object[key]);
      if (key === marker) {
        Object.defineProperty(object, "__proto__", {
          value: child,
          enumerable: true,
          configurable: true,
          writable: true,
        });
        delete object[key];
      } else object[key] = child;
    }
    return object;
  };
  return changed ? restore(value) : value;
}

export function parseConflict(raw: string): Conflict {
  const value = parseLosslessJson(raw) as JsonObject;
  if (
    !value ||
    typeof value.dataset !== "string" ||
    typeof value.featureId !== "string" ||
    !Array.isArray(value.fields)
  )
    throw new Error("冲突数据格式无效。");
  return value as Conflict;
}

export function pointerForProperty(key: string): string {
  return `/properties/${key.replaceAll("~", "~0").replaceAll("/", "~1")}`;
}

export function propertyForPointer(pointer: string): string | undefined {
  if (!pointer.startsWith("/properties/")) return;
  return pointer.slice(12).replaceAll("~1", "/").replaceAll("~0", "~");
}

export function fieldLabel(pointer: string): string {
  if (pointer === "*") return "整个要素";
  if (pointer === "/geometry") return "几何";
  return propertyForPointer(pointer) ?? pointer;
}

export function isWholeFeatureConflict(conflict: Conflict): boolean {
  return (
    conflict.reason === "stale_resolution" ||
    conflict.fields.includes("*") ||
    !conflict.base ||
    !conflict.current ||
    !conflict.draft
  );
}

function own(object: JsonObject, key: string): boolean {
  return Object.prototype.hasOwnProperty.call(object, key);
}
function same(left: unknown, right: unknown): boolean {
  return stringify(left) === stringify(right);
}
function properties(feature: ConflictFeature): JsonObject {
  return feature.properties && typeof feature.properties === "object"
    ? feature.properties
    : {};
}
function valueFor(
  feature: ConflictFeature,
  key: string,
): { present: boolean; value: unknown } {
  const source = properties(feature);
  return { present: own(source, key), value: source[key] };
}
function automatic(
  base: { present: boolean; value: unknown },
  current: { present: boolean; value: unknown },
  draft: { present: boolean; value: unknown },
): { present: boolean; value: unknown } | undefined {
  const equal = (
    left: { present: boolean; value: unknown },
    right: { present: boolean; value: unknown },
  ) =>
    left.present === right.present &&
    (!left.present || same(left.value, right.value));
  if (equal(current, draft) || equal(draft, base)) return current;
  if (equal(current, base)) return draft;
}

/** Builds an exact GeoJSON edit without turning properties into JS numbers. */
export function mergeConflict(
  conflict: Conflict,
  selections: Partial<Record<string, ConflictSide>>,
): MergedFeature | undefined {
  const whole = selections["*"];
  if (whole) {
    const feature = conflict[whole];
    return feature ? { feature, text: stringify(feature)! } : undefined;
  }
  if (isWholeFeatureConflict(conflict)) return;
  const { base, current, draft } = conflict;
  if (!base || !current || !draft) return;
  const sides: Record<ConflictSide, ConflictFeature> = { base, current, draft };
  const result: ConflictFeature = {
    ...current,
    properties: Object.create(null) as JsonObject,
  };
  const resultProperties = result.properties!;
  const keys = new Set([
    ...Object.keys(properties(base)),
    ...Object.keys(properties(current)),
    ...Object.keys(properties(draft)),
  ]);
  for (const key of keys) {
    const pointer = pointerForProperty(key);
    const side = selections[pointer];
    const selected = side
      ? valueFor(sides[side], key)
      : automatic(
          valueFor(base, key),
          valueFor(current, key),
          valueFor(draft, key),
        );
    if (!selected) return;
    if (selected.present) resultProperties[key] = selected.value;
  }
  const geometrySide = selections["/geometry"];
  const geometry = geometrySide
    ? {
        present: own(sides[geometrySide], "geometry"),
        value: sides[geometrySide].geometry,
      }
    : automatic(
        { present: own(base, "geometry"), value: base.geometry },
        { present: own(current, "geometry"), value: current.geometry },
        { present: own(draft, "geometry"), value: draft.geometry },
      );
  if (!geometry) return;
  if (geometry.present) result.geometry = geometry.value;
  return { feature: result, text: stringify(result)! };
}

export type Selection = {
  fields: Partial<Record<string, ConflictSide>>;
  text?: string;
};
/** Exact result text, `null` for a deletion, or undefined while unresolved. */
export function resolutionOf(
  conflict: Conflict,
  selection: Selection | undefined,
): string | null | undefined {
  if (!selection) return;
  if (selection.text !== undefined) return selection.text;
  const whole = selection.fields["*"];
  if (whole && !conflict[whole]) return null;
  return mergeConflict(conflict, selection.fields)?.text;
}
/** Keep one side for every conflicting part; other fields still merge three-way. */
export function sideSelection(
  conflict: Conflict,
  side: ConflictSide,
): Selection {
  if (isWholeFeatureConflict(conflict)) return { fields: { "*": side } };
  return {
    fields: Object.fromEntries(conflict.fields.map((field) => [field, side])),
  };
}
export type ChoiceKind = "draft" | "current" | "mixed" | "custom";
export function choiceKind(
  conflict: Conflict,
  selection: Selection | undefined,
): ChoiceKind | undefined {
  if (!selection || resolutionOf(conflict, selection) === undefined) return;
  if (selection.text !== undefined) return "custom";
  const whole = selection.fields["*"];
  if (whole) return whole === "base" ? "mixed" : whole;
  const sides = new Set(
    conflict.fields.map((field) => selection.fields[field]),
  );
  if (sides.size === 1) {
    const [only] = sides;
    if (only === "draft" || only === "current") return only;
  }
  return "mixed";
}
export type ConflictKind =
  | "stale"
  | "deleted-draft"
  | "deleted-current"
  | "geometry"
  | "attributes"
  | "both"
  | "feature";
export function conflictKind(conflict: Conflict): ConflictKind {
  if (conflict.reason === "stale_resolution") return "stale";
  if (!conflict.draft) return "deleted-draft";
  if (!conflict.current) return "deleted-current";
  if (isWholeFeatureConflict(conflict)) return "feature";
  const geometry = conflict.fields.includes("/geometry");
  const attributes = conflict.fields.some((field) => field !== "/geometry");
  return geometry && attributes ? "both" : geometry ? "geometry" : "attributes";
}

export type Slot = { present: boolean; value: unknown };
export type AttributeRow = {
  key: string;
  pointer: string;
  label: string;
  base: Slot;
  current: Slot;
  draft: Slot;
  /** The server reported this part as conflicting. */
  conflict: boolean;
  changed: boolean;
};
function slot(feature: ConflictFeature | null, key: string): Slot {
  if (!feature) return { present: false, value: undefined };
  if (key === "\0geometry")
    return { present: own(feature, "geometry"), value: feature.geometry };
  return valueFor(feature, key);
}
function sameSlot(left: Slot, right: Slot): boolean {
  return (
    left.present === right.present &&
    (!left.present || same(left.value, right.value))
  );
}
/** Geometry first, then every property of any side, in first-seen order. */
export function attributeRows(conflict: Conflict): AttributeRow[] {
  const sides = [conflict.base, conflict.current, conflict.draft];
  const keys: string[] = [];
  const seen = new Set<string>();
  for (const feature of sides)
    if (feature)
      for (const key of Object.keys(properties(feature)))
        if (!seen.has(key)) {
          seen.add(key);
          keys.push(key);
        }
  const whole = isWholeFeatureConflict(conflict);
  return ["\0geometry", ...keys].map((key) => {
    const geometry = key === "\0geometry";
    const pointer = geometry ? "/geometry" : pointerForProperty(key);
    const base = slot(conflict.base, key);
    const current = slot(conflict.current, key);
    const draft = slot(conflict.draft, key);
    const changed = !sameSlot(base, current) || !sameSlot(base, draft);
    return {
      key,
      pointer,
      label: geometry ? "几何" : key,
      base,
      current,
      draft,
      conflict: whole ? changed : conflict.fields.includes(pointer),
      changed,
    };
  });
}
/** The value a resolved result holds for one attribute row. */
export function resultSlot(text: string | null, row: AttributeRow): Slot {
  if (text === null) return { present: false, value: undefined };
  const feature = parseLosslessJson(text) as ConflictFeature;
  return slot(feature, row.key);
}

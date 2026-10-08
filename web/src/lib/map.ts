import type { Feature, FeatureCollection, Geometry } from "geojson";
import { isLosslessNumber, parse, stringify } from "lossless-json";

export type GeometryKind =
  "point" | "line" | "polygon" | "mixed" | "none" | "invalid";
/** Geometry classes drawn by separate map layers and shown in the legend. */
export type DrawKind = "point" | "line" | "polygon";
export const drawKinds: DrawKind[] = ["point", "line", "polygon"];
export const kindLabels: Record<GeometryKind, string> = {
  point: "点",
  line: "线",
  polygon: "面",
  mixed: "集合",
  none: "无几何",
  invalid: "坐标无效",
};
export type Bounds = [number, number, number, number];
export type PropertyRow = {
  key: string;
  value: string;
  kind: "string" | "number" | "boolean" | "null" | "object";
};
export type LayerItem = {
  /** Stable index into the loaded layer; also the MapLibre feature id. */
  index: number;
  id: string;
  raw: string;
  type: string;
  kind: GeometryKind;
  /** Geometry classes present, including the members of collections. */
  draws: DrawKind[];
  label?: string;
  bounds: Bounds | null;
  vertices: number;
  parts: number;
  properties: PropertyRow[];
  error?: string;
};
export type Layer = {
  items: LayerItem[];
  collection: FeatureCollection;
  bounds: Bounds | null;
  counts: Record<GeometryKind, number>;
  columns: string[];
};

const MAX_COLUMNS = 24;
const LABEL_KEYS = ["name", "名称", "title", "label", "NAME", "Name"];

function kindOf(type: string): DrawKind | undefined {
  if (type === "Point" || type === "MultiPoint") return "point";
  if (type === "LineString" || type === "MultiLineString") return "line";
  if (type === "Polygon" || type === "MultiPolygon") return "polygon";
  return undefined;
}

export function propertyValue(value: unknown): Omit<PropertyRow, "key"> {
  if (value === null || value === undefined)
    return { value: "null", kind: "null" };
  if (typeof value === "string") return { value, kind: "string" };
  if (typeof value === "boolean")
    return { value: String(value), kind: "boolean" };
  if (typeof value === "number" || typeof value === "bigint")
    return { value: String(value), kind: "number" };
  if (isLosslessNumber(value))
    return { value: value.toString(), kind: "number" };
  return { value: stringify(value) ?? "", kind: "object" };
}

/** Exact property rows for display; big integers keep every digit. */
export function propertyRows(raw: string): PropertyRow[] {
  try {
    const value = parse(raw) as { properties?: unknown } | null;
    const properties = value?.properties;
    if (
      !properties ||
      typeof properties !== "object" ||
      Array.isArray(properties)
    )
      return [];
    return Object.entries(properties as Record<string, unknown>).map(
      ([key, v]) => ({ key, ...propertyValue(v) }),
    );
  } catch {
    return [];
  }
}

type Stats = {
  bounds: Bounds;
  vertices: number;
  parts: number;
  draws: Set<DrawKind>;
};

function walk(geometry: Geometry, stats: Stats) {
  if (geometry.type === "GeometryCollection") {
    if (!Array.isArray(geometry.geometries))
      throw new Error("几何集合格式错误");
    for (const member of geometry.geometries) walk(member, stats);
    return;
  }
  const draw = kindOf(geometry.type);
  if (!draw) throw new Error(`不支持的几何类型 ${String(geometry.type)}`);
  const multi = geometry.type.startsWith("Multi");
  const coordinates = (value: unknown): void => {
    if (!Array.isArray(value)) throw new Error("几何坐标格式错误");
    if (!value.length) return;
    if (typeof value[0] === "number") {
      const [x, y] = value as number[];
      if (!Number.isFinite(x) || !Number.isFinite(y) || Math.abs(y) > 90)
        throw new Error("坐标不是有效的 WGS 84 经纬度");
      const b = stats.bounds;
      b[0] = Math.min(b[0], x);
      b[1] = Math.min(b[1], y);
      b[2] = Math.max(b[2], x);
      b[3] = Math.max(b[3], y);
      stats.vertices++;
    } else value.forEach(coordinates);
  };
  const before = stats.vertices;
  coordinates(geometry.coordinates);
  if (stats.vertices > before) {
    stats.draws.add(draw);
    stats.parts += multi ? (geometry.coordinates as unknown[]).length : 1;
  }
}

/** Parses one stored feature into list, inspector and map data. Never throws. */
export function layerItem(id: string, raw: string, index: number): LayerItem {
  const properties = propertyRows(raw);
  const label = properties.find(
    (p) => LABEL_KEYS.includes(p.key) && p.kind === "string" && p.value.trim(),
  )?.value;
  const base = { index, id, raw, properties, label, draws: [] as DrawKind[] };
  let geometry: Geometry | null | undefined;
  try {
    geometry = (JSON.parse(raw) as Feature | null)?.geometry;
  } catch {
    return {
      ...base,
      type: "未知",
      kind: "invalid",
      bounds: null,
      vertices: 0,
      parts: 0,
      error: "GeoJSON 无法解析",
    };
  }
  if (!geometry || typeof geometry !== "object")
    return {
      ...base,
      type: "无几何",
      kind: "none",
      bounds: null,
      vertices: 0,
      parts: 0,
    };
  const type = typeof geometry.type === "string" ? geometry.type : "未知";
  const stats: Stats = {
    bounds: [Infinity, Infinity, -Infinity, -Infinity],
    vertices: 0,
    parts: 0,
    draws: new Set(),
  };
  try {
    walk(geometry, stats);
  } catch (error) {
    return {
      ...base,
      type,
      kind: "invalid",
      bounds: null,
      vertices: 0,
      parts: 0,
      error: (error as Error).message,
    };
  }
  const draws = drawKinds.filter((k) => stats.draws.has(k));
  if (!stats.vertices)
    return { ...base, type, kind: "none", bounds: null, vertices: 0, parts: 0 };
  return {
    ...base,
    type,
    draws,
    kind:
      type === "GeometryCollection" && draws.length !== 1 ? "mixed" : draws[0],
    bounds: stats.bounds,
    vertices: stats.vertices,
    parts: stats.parts,
  };
}

export function unionBounds(list: (Bounds | null)[]): Bounds | null {
  let out: Bounds | null = null;
  for (const b of list) {
    if (!b) continue;
    out = out
      ? [
          Math.min(out[0], b[0]),
          Math.min(out[1], b[1]),
          Math.max(out[2], b[2]),
          Math.max(out[3], b[3]),
        ]
      : [...b];
  }
  return out;
}

/** Builds the layer model; the map copy carries geometry and a numeric id only. */
export function buildLayer(features: { id: string; geojson: string }[]): Layer {
  const items = features.map((f, i) => layerItem(f.id, f.geojson, i));
  const counts = {
    point: 0,
    line: 0,
    polygon: 0,
    mixed: 0,
    none: 0,
    invalid: 0,
  };
  const columns: string[] = [];
  const seen = new Set<string>();
  for (const item of items) {
    counts[item.kind]++;
    for (const p of item.properties)
      if (!seen.has(p.key) && columns.length < MAX_COLUMNS) {
        seen.add(p.key);
        columns.push(p.key);
      }
  }
  return {
    items,
    counts,
    columns,
    bounds: unionBounds(items.map((i) => i.bounds)),
    collection: mapCollection(items),
  };
}

export function mapCollection(items: LayerItem[]): FeatureCollection {
  return {
    type: "FeatureCollection",
    features: items
      .filter((i) => i.bounds)
      .map((i) => ({
        type: "Feature",
        id: i.index,
        properties: {},
        geometry: (JSON.parse(i.raw) as Feature).geometry,
      })),
  };
}

/** Case-insensitive match on feature id, property names and values. */
export function matchesQuery(item: LayerItem, query: string): boolean {
  const q = query.trim().toLowerCase();
  if (!q) return true;
  return (
    item.id.toLowerCase().includes(q) ||
    item.properties.some(
      (p) =>
        p.key.toLowerCase().includes(q) || p.value.toLowerCase().includes(q),
    )
  );
}

export function pageOf<T>(rows: T[], page: number, size: number) {
  const pages = Math.max(1, Math.ceil(rows.length / size));
  const current = Math.min(Math.max(0, page), pages - 1);
  return {
    page: current,
    pages,
    rows: rows.slice(current * size, current * size + size),
  };
}

/** Page that contains a row, for revealing a feature picked on the map. */
export function pageContaining<T>(
  rows: T[],
  match: (row: T) => boolean,
  size: number,
) {
  const at = rows.findIndex(match);
  return at < 0 ? undefined : Math.floor(at / size);
}

export function wrapLongitude(lng: number): number {
  const wrapped = ((((lng + 180) % 360) + 360) % 360) - 180;
  return wrapped === -180 && lng > 0 ? 180 : wrapped;
}

export function formatLngLat(lng: number, lat: number, digits = 5): string {
  return `${wrapLongitude(lng).toFixed(digits)}, ${lat.toFixed(digits)}`;
}

export function formatBounds(b: Bounds, digits = 5): string {
  return `${formatLngLat(b[0], b[1], digits)} – ${formatLngLat(b[2], b[3], digits)}`;
}

const MAX_LAT = 85.051129;
/** Clamps latitudes to the Web Mercator range so polar data can still be fitted. */
export function mercatorBounds(b: Bounds): Bounds {
  const lat = (y: number) => Math.max(-MAX_LAT, Math.min(MAX_LAT, y));
  return [b[0], lat(b[1]), b[2], lat(b[3])];
}

export function contains(outer: Bounds, inner: Bounds): boolean {
  return (
    inner[0] >= outer[0] &&
    inner[1] >= outer[1] &&
    inner[2] <= outer[2] &&
    inner[3] <= outer[3]
  );
}

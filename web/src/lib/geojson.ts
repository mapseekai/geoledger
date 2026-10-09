import type { Feature, Geometry } from "geojson";
import { stringify } from "lossless-json";
import { parseLosslessJson } from "./conflicts";
export function pretty(raw: string): string {
  try {
    return stringify(parseLosslessJson(raw), undefined, 2) ?? raw;
  } catch {
    return raw;
  }
}
export function featureText(raw: string, id: string): string {
  const value = parseLosslessJson(raw) as Record<string, unknown> | null;
  if (
    !value ||
    typeof value !== "object" ||
    Array.isArray(value) ||
    value.type !== "Feature"
  )
    throw new Error("请输入 GeoJSON Feature 对象。");
  if (value.id !== undefined && value.id !== id)
    throw new Error("要素标识与 GeoJSON 中的 id 不一致。");
  value.id = id;
  return stringify(value)!;
}
/** Geometry type for list display; reads with lossless-json and never throws. */
export function geometryType(raw: string): string {
  try {
    const value = parseLosslessJson(raw) as {
      geometry?: { type?: unknown } | null;
    } | null;
    const type = value?.geometry?.type;
    return typeof type === "string" ? type : "无几何";
  } catch {
    return "未知";
  }
}

// The rendering copy contains geometry only; exact properties stay in the raw text.
export function previewGeometry(raw: string) {
  const feature = JSON.parse(raw) as Feature;
  if (feature?.type !== "Feature")
    throw new Error("请输入 GeoJSON Feature 对象。");
  const bounds: [number, number, number, number] = [
    Infinity,
    Infinity,
    -Infinity,
    -Infinity,
  ];
  function coordinates(value: unknown): void {
    if (!Array.isArray(value)) throw new Error("几何坐标格式错误。");
    if (!value.length) return;
    if (typeof value[0] === "number") {
      const [x, y] = value;
      if (!Number.isFinite(x) || !Number.isFinite(y) || Math.abs(y) > 90)
        throw new Error("地图预览需要有效的经纬度坐标。");
      bounds[0] = Math.min(bounds[0], x);
      bounds[1] = Math.min(bounds[1], y);
      bounds[2] = Math.max(bounds[2], x);
      bounds[3] = Math.max(bounds[3], y);
    } else value.forEach(coordinates);
  }
  function visit(geometry: Geometry | null): void {
    if (!geometry) return;
    if (geometry.type === "GeometryCollection")
      geometry.geometries.forEach(visit);
    else coordinates(geometry.coordinates);
  }
  visit(feature.geometry);
  return {
    data: {
      type: "Feature",
      properties: {},
      geometry: feature.geometry,
    } as Feature,
    bounds: Number.isFinite(bounds[0]) ? bounds : null,
  };
}

import { stringify } from "lossless-json";

const integer = new Intl.NumberFormat("zh-CN");
export function count(value: string | number | bigint | undefined): string {
  if (value === undefined) return "–";
  try {
    return integer.format(BigInt(value));
  } catch {
    return String(value);
  }
}
export function fullTime(value: string): string {
  const date = new Date(value);
  return Number.isNaN(date.getTime())
    ? value
    : date.toLocaleString("zh-CN", { hour12: false });
}
const relative = new Intl.RelativeTimeFormat("zh-CN", { numeric: "auto" });
export function relativeTime(value: string, now = Date.now()): string {
  const date = new Date(value).getTime();
  if (Number.isNaN(date)) return value;
  const seconds = Math.round((date - now) / 1000);
  const abs = Math.abs(seconds);
  if (abs < 45) return "刚刚";
  const units: [Intl.RelativeTimeFormatUnit, number][] = [
    ["minute", 60],
    ["hour", 3600],
    ["day", 86400],
    ["month", 2592000],
    ["year", 31536000],
  ];
  let unit = units[0];
  for (const candidate of units)
    if (abs >= candidate[1] * 0.9) unit = candidate;
  if (unit[0] === "day" && abs >= 86400 * 7 && abs < 2592000 * 0.9)
    return fullTime(value).split(" ")[0];
  return relative.format(Math.round(seconds / unit[1]), unit[0]);
}
type Geometry = { type?: unknown; coordinates?: unknown; geometries?: unknown };
function points(value: unknown): number {
  if (!Array.isArray(value)) return 0;
  // A position is an array of numbers (lossless numbers are objects, not arrays).
  if (value.length && !Array.isArray(value[0])) return 1;
  return value.reduce((sum: number, item) => sum + points(item), 0);
}
/** Short geometry label such as "LineString · 3 点". */
export function geometryLabel(geometry: unknown): string {
  if (geometry === null || geometry === undefined) return "无几何";
  const g = geometry as Geometry;
  if (typeof g.type !== "string") return "无效几何";
  if (g.type === "GeometryCollection")
    return `GeometryCollection · ${Array.isArray(g.geometries) ? g.geometries.length : 0} 个`;
  if (g.type === "Point" && Array.isArray(g.coordinates))
    return `Point (${g.coordinates
      .slice(0, 2)
      .map((n) => Number(String(n)).toFixed(4))
      .join(", ")})`;
  return `${g.type} · ${points(g.coordinates)} 点`;
}
/** Compact display text for one attribute value; exact numbers are preserved. */
export function valueText(present: boolean, value: unknown, geometry = false) {
  if (!present) return "";
  if (geometry) return geometryLabel(value);
  if (typeof value === "string") return value;
  return stringify(value) ?? "null";
}

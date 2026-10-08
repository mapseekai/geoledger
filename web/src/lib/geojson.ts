import { parse, stringify } from "lossless-json";
export function pretty(raw: string): string {
  try {
    return stringify(parse(raw), undefined, 2) ?? raw;
  } catch {
    return raw;
  }
}
export function featureText(raw: string, id: string): string {
  const value = parse(raw) as Record<string, unknown> | null;
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

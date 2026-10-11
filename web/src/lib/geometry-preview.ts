/** Projects GeoJSON geometries into one SVG viewport; no tiles or network. */
export type Shape =
  | { kind: "point"; x: number; y: number }
  | { kind: "path"; d: string; closed: boolean };
type Position = [number, number];
type Geometry = { type?: unknown; coordinates?: unknown; geometries?: unknown };

const num = (value: unknown) => Number(String(value));
function position(value: unknown): Position | undefined {
  if (!Array.isArray(value) || value.length < 2) return;
  const x = num(value[0]),
    y = num(value[1]);
  return Number.isFinite(x) && Number.isFinite(y) ? [x, y] : undefined;
}
type Part = { kind: "point" | "line" | "ring"; positions: Position[] };
function parts(geometry: unknown, out: Part[] = []): Part[] {
  const g = geometry as Geometry | null;
  if (!g || typeof g !== "object") return out;
  const c = g.coordinates as unknown[];
  const line = (items: unknown, kind: "line" | "ring") => {
    if (!Array.isArray(items)) return;
    const positions = items.map(position).filter(Boolean) as Position[];
    if (positions.length) out.push({ kind, positions });
  };
  switch (g.type) {
    case "Point": {
      const p = position(c);
      if (p) out.push({ kind: "point", positions: [p] });
      break;
    }
    case "MultiPoint":
      for (const item of c ?? [])
        parts({ type: "Point", coordinates: item }, out);
      break;
    case "LineString":
      line(c, "line");
      break;
    case "MultiLineString":
      for (const item of c ?? []) line(item, "line");
      break;
    case "Polygon":
      for (const item of c ?? []) line(item, "ring");
      break;
    case "MultiPolygon":
      for (const polygon of c ?? [])
        for (const ring of (polygon as unknown[]) ?? []) line(ring, "ring");
      break;
    case "GeometryCollection":
      for (const item of (g.geometries as unknown[]) ?? []) parts(item, out);
      break;
  }
  return out;
}
export function previewShapes(
  geometries: unknown[],
  size: { width: number; height: number; padding: number },
): Shape[][] {
  const all = geometries.map((geometry) => parts(geometry));
  const flat = all.flat().flatMap((part) => part.positions);
  if (!flat.length) return all.map(() => []);
  let [minX, minY] = flat[0],
    [maxX, maxY] = flat[0];
  for (const [x, y] of flat) {
    minX = Math.min(minX, x);
    maxX = Math.max(maxX, x);
    minY = Math.min(minY, y);
    maxY = Math.max(maxY, y);
  }
  // Equirectangular with latitude correction keeps small areas undistorted.
  const k = Math.cos((((minY + maxY) / 2) * Math.PI) / 180) || 1;
  const spanX = Math.max((maxX - minX) * k, 1e-9),
    spanY = Math.max(maxY - minY, 1e-9);
  const innerW = size.width - size.padding * 2,
    innerH = size.height - size.padding * 2;
  const scale = Math.min(innerW / spanX, innerH / spanY);
  const offsetX = size.padding + (innerW - spanX * scale) / 2,
    offsetY = size.padding + (innerH - spanY * scale) / 2;
  const project = ([x, y]: Position): Position => [
    +(offsetX + (x - minX) * k * scale).toFixed(2),
    +(offsetY + (maxY - y) * scale).toFixed(2),
  ];
  return all.map((list) =>
    list.map((part) => {
      if (part.kind === "point") {
        const [x, y] = project(part.positions[0]);
        return { kind: "point" as const, x, y };
      }
      const d = part.positions
        .map((p, i) => `${i ? "L" : "M"}${project(p).join(" ")}`)
        .join(" ");
      return {
        kind: "path" as const,
        d: part.kind === "ring" ? `${d} Z` : d,
        closed: part.kind === "ring",
      };
    }),
  );
}

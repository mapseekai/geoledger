import { isLosslessNumber, stringify } from "lossless-json";
import type { GeometryType } from "./browser-api";
import { parseLosslessJson } from "./conflicts";

export type ImportedFeature = {
  id: string;
  raw: string;
  crs?: string;
};

const object = (value: unknown): value is Record<string, unknown> =>
  !!value && typeof value === "object" && !Array.isArray(value);
const geometryTypes = new Set([
  "Point",
  "MultiPoint",
  "LineString",
  "MultiLineString",
  "Polygon",
  "MultiPolygon",
  "GeometryCollection",
]);

export function normalizeCrs(value: string): string {
  const name = value.trim();
  const epsg =
    /^(?:EPSG:|URN:OGC:DEF:CRS:EPSG::|HTTPS?:\/\/WWW\.OPENGIS\.NET\/DEF\/CRS\/EPSG\/0\/)(\d+)$/i.exec(
      name,
    );
  if (epsg) return `EPSG:${Number(epsg[1])}`;
  if (
    /^(?:OGC:CRS84|URN:OGC:DEF:CRS:OGC:1\.3:CRS84|HTTPS?:\/\/WWW\.OPENGIS\.NET\/DEF\/CRS\/OGC\/1\.3\/CRS84)$/i.test(
      name,
    )
  )
    return "EPSG:4326";
  return name;
}

function declaredCrs(value: unknown, inherited: string): string {
  if (value === undefined || value === null) return inherited;
  if (
    object(value) &&
    value.type === "name" &&
    object(value.properties) &&
    typeof value.properties.name === "string"
  ) {
    const name = normalizeCrs(value.properties.name);
    if (name && new TextEncoder().encode(name).length <= 16384) return name;
  }
  throw new Error("crs 必须声明非空坐标系名称或定义。");
}

function coordinates(value: unknown): unknown {
  if (!Array.isArray(value) || !value.length)
    throw new Error("coordinates 必须是非空坐标数组。");
  if (Array.isArray(value[0])) return value.map((v) => coordinates(v));
  if (
    value.length < 2 ||
    value.length > 3 ||
    !value.every(
      (v) =>
        (typeof v === "number" || isLosslessNumber(v)) &&
        Number.isFinite(Number(String(v))),
    )
  )
    throw new Error("坐标必须包含两个或三个有限数值。");
  return value;
}

export const geometryLabels = { point: "点", line: "线", polygon: "面" };
export function assertGeometryType(value: unknown, family: GeometryType): void {
  if (value === null) return;
  if (
    object(value) &&
    value.type === "GeometryCollection" &&
    Array.isArray(value.geometries)
  ) {
    value.geometries.forEach((g) => assertGeometryType(g, family));
    return;
  }
  const allowed = {
    point: ["Point", "MultiPoint"],
    line: ["LineString", "MultiLineString"],
    polygon: ["Polygon", "MultiPolygon"],
  };
  if (!object(value) || !allowed[family].includes(String(value.type)))
    throw new Error(`此数据集只能存放${geometryLabels[family]}类型要素。`);
}

export function assertCoordinateDimension(
  value: unknown,
  dimension: 2 | 3,
): void {
  if (value === null) return;
  if (Array.isArray(value)) {
    if (
      value.length &&
      (typeof value[0] === "number" || isLosslessNumber(value[0]))
    ) {
      if (value.length !== dimension)
        throw new Error(
          `此数据集要求${dimension === 2 ? "二维（XY）" : "三维（XYZ）"}坐标。`,
        );
    } else value.forEach((part) => assertCoordinateDimension(part, dimension));
  } else if (object(value)) {
    if ("coordinates" in value)
      assertCoordinateDimension(value.coordinates, dimension);
    if ("geometries" in value)
      assertCoordinateDimension(value.geometries, dimension);
  }
}

/** Parse without rounding property values or numeric feature identifiers. */
export function importGeojson(
  raw: string,
  createId: () => string = () => crypto.randomUUID(),
  family?: GeometryType,
  dimension?: 2 | 3,
  expectedCrs?: string,
): ImportedFeature[] {
  let root: unknown;
  try {
    root = parseLosslessJson(raw.replace(/^\uFEFF/, ""));
  } catch {
    throw new Error("文件不是有效的 JSON，请选择标准 GeoJSON 文件。");
  }
  if (!object(root)) throw new Error("文件必须包含 GeoJSON 对象。");
  const rootCrs = declaredCrs(
    root.crs,
    expectedCrs ? normalizeCrs(expectedCrs) : "EPSG:4326",
  );
  if (expectedCrs && rootCrs !== normalizeCrs(expectedCrs))
    throw new Error(
      "文件坐标系与数据集不一致；请先转换坐标或创建对应坐标系的数据集。",
    );
  const features =
    root.type === "FeatureCollection"
      ? root.features
      : root.type === "Feature"
        ? [root]
        : geometryTypes.has(String(root.type))
          ? [{ type: "Feature", properties: {}, geometry: root }]
          : null;
  if (!Array.isArray(features) || !features.length)
    throw new Error("请选择 Feature、非空 FeatureCollection 或几何对象。");
  const ids = new Set<string>();
  return features.map((feature, index) => {
    const fail = (message: string): never => {
      throw new Error(`第 ${index + 1} 个要素：${message}`);
    };
    if (!object(feature) || feature.type !== "Feature")
      return fail("必须是 Feature 对象。");
    if (feature.properties !== null && !object(feature.properties))
      return fail("properties 必须是对象或 null。");
    if (
      feature.geometry !== null &&
      (!object(feature.geometry) ||
        !geometryTypes.has(String(feature.geometry.type)))
    )
      return fail("geometry 必须是有效的几何对象或 null。");
    if (family) {
      try {
        assertGeometryType(feature.geometry, family);
      } catch (e) {
        return fail((e as Error).message);
      }
    }
    if (dimension) {
      try {
        assertCoordinateDimension(feature.geometry, dimension);
      } catch (e) {
        return fail((e as Error).message);
      }
    }
    const originalId = feature.id;
    if (
      originalId !== undefined &&
      typeof originalId !== "string" &&
      !isLosslessNumber(originalId)
    )
      return fail("id 必须是字符串或数字。");
    const id = originalId === undefined ? createId() : String(originalId);
    if (!id.trim() || new TextEncoder().encode(id).length > 256)
      return fail("标识不能为空或超过 256 字节。");
    if (ids.has(id)) return fail(`标识 ${id} 重复。`);
    ids.add(id);
    const featureCrs = declaredCrs(feature.crs, rootCrs);
    if (featureCrs !== rootCrs) return fail("一个数据集不能混用不同坐标系。");
    // Adapt standard GeoJSON to the current storage contract.
    const geometry = (value: unknown, inherited = featureCrs): unknown => {
      if (value === null) return null;
      if (!object(value) || !geometryTypes.has(String(value.type)))
        return fail("无效的几何对象。");
      const crs = declaredCrs(value.crs, inherited);
      if (crs !== rootCrs) return fail("几何坐标系与数据集不一致。");
      if (value.type === "GeometryCollection") {
        if (!Array.isArray(value.geometries))
          return fail("geometries 必须是数组。");
        return {
          type: value.type,
          geometries: value.geometries.map((g) => geometry(g, crs)),
        };
      }
      if (!Array.isArray(value.coordinates))
        return fail("coordinates 必须是数组。");
      try {
        return {
          type: value.type,
          coordinates: coordinates(value.coordinates),
        };
      } catch (error) {
        return fail(error instanceof Error ? error.message : "无效坐标。");
      }
    };
    const text = stringify({
      type: "Feature",
      id,
      properties: feature.properties ?? {},
      geometry: geometry(feature.geometry),
    })!;
    return { id, raw: text, crs: rootCrs };
  });
}

/** Bound batches by payload size, without imposing a per-feature size cap. */
export function importBatch(
  rows: ImportedFeature[],
  offset: number,
): ImportedFeature[] {
  let bytes = 0;
  let end = offset;
  const encoder = new TextEncoder();
  while (end < rows.length && end - offset < 20) {
    const size = encoder.encode(JSON.stringify(rows[end])).length;
    if (end > offset && bytes + size > 512 * 1024) break;
    bytes += size;
    end++;
  }
  return rows.slice(offset, end);
}

/** Infer the immutable dataset shape from all non-null geometries before creation. */
export function importedShape(
  rows: ImportedFeature[],
): { geometryType: GeometryType; coordinateDimension: 2 | 3 } | undefined {
  const families = new Set<GeometryType>();
  const dimensions = new Set<2 | 3>();
  function visit(value: unknown) {
    if (!object(value)) return;
    if (value.type === "GeometryCollection") {
      (value.geometries as unknown[]).forEach(visit);
      return;
    }
    const family = String(value.type).includes("Point")
      ? "point"
      : String(value.type).includes("LineString")
        ? "line"
        : "polygon";
    families.add(family);
    function dimension(v: unknown) {
      if (!Array.isArray(v) || !v.length) return;
      if (Array.isArray(v[0])) v.forEach(dimension);
      else dimensions.add(v.length as 2 | 3);
    }
    dimension(value.coordinates);
  }
  rows.forEach((row) =>
    visit((parseLosslessJson(row.raw) as { geometry: unknown }).geometry),
  );
  if (families.size > 1 || dimensions.size > 1)
    throw new Error(
      "请按点、线、面以及 XY、XYZ 分成独立文件，每个文件创建一个数据集。",
    );
  if (!families.size) return undefined;
  return {
    geometryType: [...families][0],
    coordinateDimension: [...dimensions][0] ?? 2,
  };
}

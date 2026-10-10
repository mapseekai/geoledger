import { LosslessNumber, compareLosslessNumber } from "lossless-json";
import { call, type FeaturePage, type Workspace } from "./browser-api";
import { parseLosslessJson } from "./conflicts";
import type { ImportedFeature } from "./geojson-import";

function same(left: unknown, right: unknown): boolean {
  if (left instanceof LosslessNumber || right instanceof LosslessNumber) {
    if (!(left instanceof LosslessNumber) || !(right instanceof LosslessNumber))
      return false;
    if (compareLosslessNumber(left, right) === 0) return true;
    // Match the server's binary64 decimal contract while keeping integers exact.
    const a = Number(left.value);
    const b = Number(right.value);
    return a !== 0 && Math.abs(a) < 2 ** 53 && a === b;
  }
  if (Array.isArray(left) || Array.isArray(right))
    return (
      Array.isArray(left) &&
      Array.isArray(right) &&
      left.length === right.length &&
      left.every((value, index) => same(value, right[index]))
    );
  if (left && right && typeof left === "object" && typeof right === "object") {
    const other = right as Record<string, unknown>;
    return (
      Object.keys(left).length === Object.keys(right).length &&
      Object.entries(left).every(
        ([key, value]) => Object.hasOwn(other, key) && same(value, other[key]),
      )
    );
  }
  return left === right;
}

/** Confirm an uncertain save without replacing concurrent workspace edits. */
export async function recoverImportBatch(
  project: string,
  dataset: string,
  previous: Workspace,
  batch: ImportedFeature[],
  request: typeof call = call,
): Promise<Workspace | undefined> {
  const current = await request<Workspace>({
    action: "workspace",
    project,
    workspace: previous.id,
  });
  const conflict = () =>
    new Error(
      "工作区已发生其他修改，请检查工作区中的要素后再处理导入；已保存的数据会保留。",
    );
  if (
    current.status !== "open" ||
    current.baseRevision !== previous.baseRevision
  )
    throw conflict();
  if (current.version === previous.version) return undefined;
  if (BigInt(current.version) !== BigInt(previous.version) + 1n)
    throw conflict();
  for (const row of batch) {
    const page = await request<FeaturePage>({
      action: "features",
      project,
      dataset,
      workspace: current.id,
      workspaceVersion: current.version,
      featureId: row.id,
      limit: 1,
    });
    if (
      page.workspaceVersion !== current.version ||
      page.features.length !== 1 ||
      page.features[0].id !== row.id ||
      !same(
        parseLosslessJson(page.features[0].geojson),
        parseLosslessJson(row.raw),
      )
    )
      throw conflict();
  }
  return current;
}

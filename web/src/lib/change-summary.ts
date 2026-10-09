export type ChangeCounts = { added: number; deleted: number; modified: number };
export type ChangeSummary = {
  total: ChangeCounts;
  datasets: (ChangeCounts & { id: string; name: string })[];
};
type Change = {
  cursor: string;
  dataset: string;
  base: unknown;
  draft: unknown;
  before: unknown;
  after: unknown;
};
export class SummaryVersionError extends Error {
  constructor() {
    super("工作区已变化，请刷新后重新统计。");
  }
}

/** Count persisted feature differences, never individual property edits. */
export async function summarizeChanges(
  kind: "workspace" | "commit",
  load: (
    after: string,
    limit: number,
  ) => Promise<{ changes: Change[]; version?: bigint }>,
  names: Map<string, string>,
  expectedVersion?: bigint,
): Promise<ChangeSummary> {
  const total: ChangeCounts = { added: 0, deleted: 0, modified: 0 };
  const datasets = new Map<string, ChangeSummary["datasets"][number]>();
  let after = "";
  let limit = 20;
  for (;;) {
    let page;
    try {
      page = await load(after, limit);
    } catch (error) {
      if (
        error &&
        typeof error === "object" &&
        "code" in error &&
        error.code === "resource_exhausted" &&
        limit > 1
      ) {
        limit = Math.max(1, Math.floor(limit / 2));
        continue;
      }
      throw error;
    }
    if (kind === "workspace" && page.version !== expectedVersion)
      throw new SummaryVersionError();
    for (const change of page.changes) {
      const before = kind === "workspace" ? change.base : change.before;
      const after = kind === "workspace" ? change.draft : change.after;
      if (before == null && after == null) continue;
      const field =
        before == null ? "added" : after == null ? "deleted" : "modified";
      let counts = datasets.get(change.dataset);
      if (!counts) {
        counts = {
          id: change.dataset,
          name: names.get(change.dataset) ?? change.dataset,
          added: 0,
          deleted: 0,
          modified: 0,
        };
        datasets.set(change.dataset, counts);
      }
      counts[field]++;
      total[field]++;
    }
    if (page.changes.length < limit) break;
    const next = page.changes.at(-1)!.cursor;
    if (next === after) throw new Error("变更分页游标未前进，请刷新重试。");
    after = next;
  }
  return { total, datasets: [...datasets.values()] };
}

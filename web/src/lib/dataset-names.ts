import { call, type Dataset } from "./browser-api";

/** Cosmetic names only: preserve the existing twenty-page bound and opaque cursor. */
export async function loadDatasetNames(
  project: string,
  signal: AbortSignal,
): Promise<Map<string, string>> {
  const names = new Map<string, string>();
  let after = "";
  for (let page = 0; page < 20; page++) {
    signal.throwIfAborted();
    const rows = await call<Dataset[]>(
      { action: "datasets", project, after: after || undefined, limit: 100 },
      signal,
    );
    signal.throwIfAborted();
    for (const row of rows) names.set(row.id, row.name);
    if (rows.length < 100) break;
    after = rows.at(-1)!.id;
  }
  return names;
}

import type { SchemaChange } from "@geoledger/client";

export type ChangeCounts = {
  added: string | number;
  deleted: string | number;
  modified: string | number;
};
export type ChangeSummary = {
  total: ChangeCounts;
  schemaChanges?: SchemaChange[];
  datasets: (ChangeCounts & { id: string; name: string })[];
  version?: string;
  revision?: string;
};

export function hasFeatureChanges(total?: ChangeCounts): boolean {
  return (
    !!total &&
    [total.added, total.modified, total.deleted].some((n) => String(n) !== "0")
  );
}

/** Unknown summaries are not an empty-workspace result. */
export function isEmptySummary(summary?: ChangeSummary): boolean {
  return (
    !!summary &&
    !hasFeatureChanges(summary.total) &&
    !summary.schemaChanges?.length
  );
}

export function schemaFieldChanges(change: SchemaChange) {
  const before = new Map(change.before.map((field) => [field.name, field]));
  const after = new Map(change.after.map((field) => [field.name, field]));
  return {
    added: change.after
      .filter((field) => !before.has(field.name))
      .map((field) => field.name),
    deleted: change.before
      .filter((field) => !after.has(field.name))
      .map((field) => field.name),
    modified: change.after
      .filter((field) => {
        const original = before.get(field.name);
        return (
          original &&
          (original.type !== field.type ||
            original.nullable !== field.nullable ||
            original.editable !== field.editable)
        );
      })
      .map((field) => field.name),
  };
}

/** Coalesce simultaneous consumers only. Never reuse results across mutations or sessions. */
export function sharedRequests<T>() {
  const pending = new Map<string, Promise<T>>();
  return (key: string, load: () => Promise<T>): Promise<T> => {
    const existing = pending.get(key);
    if (existing) return existing;
    const request = Promise.resolve()
      .then(load)
      .finally(() => pending.delete(key));
    pending.set(key, request);
    return request;
  };
}

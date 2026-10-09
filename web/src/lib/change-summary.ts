export type ChangeCounts = {
  added: string | number;
  deleted: string | number;
  modified: string | number;
};
export type ChangeSummary = {
  total: ChangeCounts;
  datasets: (ChangeCounts & { id: string; name: string })[];
  version?: string;
  revision?: string;
};

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

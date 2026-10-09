export function featureItemsUrl(query: Record<string, unknown>) {
  const base = `/api/projects/${encodeURIComponent(String(query.project))}/collections/${encodeURIComponent(String(query.dataset))}/items`;
  const params = new URLSearchParams();
  for (const key of [
    "limit",
    "after",
    "revision",
    "workspace",
    "workspaceVersion",
    "featureId",
    "bbox",
  ]) {
    const value = query[key];
    if (value !== undefined && value !== null)
      params.set(key, Array.isArray(value) ? value.join(",") : String(value));
  }
  return params.size ? `${base}?${params}` : base;
}

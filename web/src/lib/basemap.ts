/** Optional raster basemap configured by the operator (XYZ tile URL template). */
export type RasterBasemap = {
  url: string;
  origin: string;
  attribution: string;
};

/**
 * Reads GL_WEB_BASEMAP_URL / GL_WEB_BASEMAP_ATTRIBUTION. Returns undefined when unset
 * or invalid, so the console keeps its offline blank basemaps and strict CSP.
 */
export function rasterBasemap(
  env: Record<string, string | undefined>,
): RasterBasemap | undefined {
  const url = env.GL_WEB_BASEMAP_URL?.trim();
  if (!url || !["{z}", "{x}", "{y}"].every((token) => url.includes(token)))
    return undefined;
  let parsed: URL;
  try {
    parsed = new URL(url.replace(/\{[a-z-]+\}/gi, "0"));
  } catch {
    return undefined;
  }
  if (
    !["http:", "https:"].includes(parsed.protocol) ||
    parsed.username ||
    parsed.password ||
    /[\s'";,]/.test(url)
  )
    return undefined;
  return {
    url,
    origin: parsed.origin,
    attribution:
      env.GL_WEB_BASEMAP_ATTRIBUTION?.trim().slice(0, 200) || parsed.hostname,
  };
}

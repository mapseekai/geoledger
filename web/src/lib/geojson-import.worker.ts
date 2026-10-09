import { importGeojson } from "./geojson-import";
import type { GeometryType } from "./browser-api";

self.onmessage = async (
  event: MessageEvent<{ file: File; family: GeometryType }>,
) => {
  try {
    const rows = importGeojson(
      await event.data.file.text(),
      undefined,
      event.data.family,
    );
    self.postMessage({ rows });
  } catch (error) {
    self.postMessage({
      error: error instanceof Error ? error.message : "无法读取 GeoJSON 文件。",
    });
  }
};

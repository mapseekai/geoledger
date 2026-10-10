import { importGeojson, importedShape } from "./geojson-import";
import type { GeometryType } from "./browser-api";

self.onmessage = async (
  event: MessageEvent<{ file: File; family?: GeometryType; dimension?: 2 | 3 }>,
) => {
  try {
    const text = await event.data.file.text();
    if (event.data.file.size > 0 && text.length === 0) {
      throw new Error(
        "浏览器读取文件失败：未读取到文件内容，请重试或选择较小的文件。",
      );
    }
    const rows = importGeojson(
      text,
      undefined,
      event.data.family,
      event.data.dimension,
    );
    self.postMessage({ rows, shape: importedShape(rows) });
  } catch (error) {
    self.postMessage({
      error: error instanceof Error ? error.message : "无法读取 GeoJSON 文件。",
    });
  }
};

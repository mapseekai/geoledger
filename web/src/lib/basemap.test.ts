import assert from "node:assert/strict";
import test from "node:test";
import { rasterBasemap } from "./basemap";

test("raster basemap is opt-in and only accepts HTTP(S) XYZ templates", () => {
  assert.equal(rasterBasemap({}), undefined);
  assert.deepEqual(
    rasterBasemap({
      GL_WEB_BASEMAP_URL: "https://tile.openstreetmap.org/{z}/{x}/{y}.png",
      GL_WEB_BASEMAP_ATTRIBUTION: "© OpenStreetMap contributors",
    }),
    {
      url: "https://tile.openstreetmap.org/{z}/{x}/{y}.png",
      origin: "https://tile.openstreetmap.org",
      attribution: "© OpenStreetMap contributors",
    },
  );
  assert.equal(
    rasterBasemap({
      GL_WEB_BASEMAP_URL: "http://10.0.0.5:8080/{z}/{x}/{y}.png",
    })?.attribution,
    "10.0.0.5",
  );
  for (const url of [
    "https://tiles.example/{z}/{x}.png",
    "ftp://tiles.example/{z}/{x}/{y}.png",
    "javascript:alert(1)//{z}{x}{y}",
    "https://tiles.example/{z}/{x}/{y}.png; script-src *",
    "https://user:pw@tiles.example/{z}/{x}/{y}.png",
  ])
    assert.equal(rasterBasemap({ GL_WEB_BASEMAP_URL: url }), undefined, url);
});

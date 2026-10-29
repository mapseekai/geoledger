import assert from "node:assert/strict";
import test from "node:test";
import { count, geometryLabel, relativeTime } from "./format";

test("formatting helpers", () => {
  assert.equal(count("1234567"), "1,234,567");
  assert.equal(count(undefined), "–");
  assert.equal(
    geometryLabel({
      type: "LineString",
      coordinates: [
        [0, 0],
        [1, 1],
        [2, 2],
      ],
    }),
    "LineString · 3 点",
  );
  assert.equal(
    geometryLabel({
      type: "Polygon",
      coordinates: [
        [
          [0, 0],
          [1, 0],
          [1, 1],
          [0, 0],
        ],
      ],
    }),
    "Polygon · 4 点",
  );
  assert.equal(geometryLabel(null), "无几何");
  const now = Date.parse("2026-10-10T12:00:00Z");
  assert.equal(relativeTime("2026-10-10T11:59:50Z", now), "刚刚");
  assert.equal(relativeTime("2026-10-10T11:55:00Z", now), "5分钟前");
  assert.equal(relativeTime("2026-10-10T09:00:00Z", now), "3小时前");
});

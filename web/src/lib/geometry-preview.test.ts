import assert from "node:assert/strict";
import test from "node:test";
import { previewShapes } from "./geometry-preview";

test("geometry preview projects all versions into one viewport", () => {
  const [a, b] = previewShapes(
    [
      { type: "Point", coordinates: [104, 30] },
      {
        type: "LineString",
        coordinates: [
          [104, 30],
          [104.01, 30.01],
        ],
      },
    ],
    { width: 100, height: 100, padding: 10 },
  );
  assert.equal(a[0].kind, "point");
  assert.equal(b[0].kind, "path");
  const [empty] = previewShapes([null], {
    width: 100,
    height: 100,
    padding: 10,
  });
  assert.deepEqual(empty, []);
});

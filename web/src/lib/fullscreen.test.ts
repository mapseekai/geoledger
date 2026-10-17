import assert from "node:assert/strict";
import test from "node:test";
import { leaveFullscreen, toggleFullscreen } from "./fullscreen";

test("fullscreen rejection is reported for user actions and ignored for best-effort exit", async () => {
  const original = Object.getOwnPropertyDescriptor(globalThis, "document");
  let fullscreenElement: object | null = null;
  let reports = 0;
  const denied = () => Promise.reject(new Error("fullscreen denied"));
  Object.defineProperty(globalThis, "document", {
    configurable: true,
    value: {
      get fullscreenElement() {
        return fullscreenElement;
      },
      exitFullscreen: denied,
    },
  });
  try {
    await toggleFullscreen({ requestFullscreen: denied }, () => reports++);
    assert.equal(reports, 1);
    fullscreenElement = {};
    await toggleFullscreen(null, () => reports++);
    assert.equal(reports, 2);
    await leaveFullscreen();
    assert.equal(reports, 2);
    fullscreenElement = null;
    await toggleFullscreen(null, () => reports++);
    assert.equal(reports, 2);
  } finally {
    if (original) Object.defineProperty(globalThis, "document", original);
    else Reflect.deleteProperty(globalThis, "document");
  }
});

import assert from "node:assert/strict";
import test from "node:test";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { Progress } from "./progress";

test("progress exposes its numeric value to assistive technology", () => {
  for (const value of [0, 50, 100]) {
    const html = renderToStaticMarkup(createElement(Progress, { value }));
    assert.match(html, new RegExp(`aria-valuenow="${value}"`));
    assert.match(html, /aria-valuemax="100"/);
  }
});

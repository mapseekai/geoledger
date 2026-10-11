import assert from "node:assert/strict";
import { test } from "node:test";
import { mkdtempSync, writeFileSync, rmSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { join } from "node:path";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { Progress } from "../src/components/ui/progress";

test("progress exposes its numeric value to assistive technology", () => {
  for (const value of [0, 50, 100]) {
    const html = renderToStaticMarkup(createElement(Progress, { value }));
    assert.match(html, new RegExp(`aria-valuenow="${value}"`));
    assert.match(html, /aria-valuemax="100"/);
  }
});

test("design policy accepts supported styles and still rejects violations", () => {
  const dir = mkdtempSync(join(process.cwd(), "tests/lint-fixture-"));
  const file = join(dir, "fixture.tsx");
  const lint = (source: string) => {
    writeFileSync(file, source);
    const result = spawnSync(
      process.execPath,
      ["node_modules/oxlint/bin/oxlint", file, "--format", "json"],
      { encoding: "utf8" },
    );
    assert.ifError(result.error);
    const output = JSON.parse(result.stdout);
    assert.equal(output.number_of_files, 1);
    return output.diagnostics as { code: string }[];
  };
  try {
    assert.deepEqual(
      lint(`import { Button } from "@/components/ui/button";
      export const Good = () => <Button className="w-full source-trigger">Open</Button>;`),
      [],
    );
    const diagnostics = lint(`import { Button } from "@/components/ui/button";
      export const Bad = ({ extra }: { extra: string }) => <><Button className="p-[13px] text-red-500 rounded-huge" style={{ opacity: 0.5 }}>Open</Button><Button className={extra}>Dynamic</Button></>;`);
    for (const rule of [
      "no-restyle",
      "no-raw-colors",
      "no-arbitrary-values",
      "no-inline-styles",
      "no-unknown-classes",
      "require-static-classes",
    ])
      assert(
        diagnostics.some(({ code }) => code === `shadcn(${rule})`),
        rule,
      );
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

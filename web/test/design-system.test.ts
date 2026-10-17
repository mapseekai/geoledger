import assert from "node:assert/strict";
import { test } from "node:test";
import {
  mkdtempSync,
  writeFileSync,
  rmSync,
  readdirSync,
  readFileSync,
} from "node:fs";
import { spawnSync } from "node:child_process";
import { join } from "node:path";

test("design policy accepts supported styles and still rejects violations", () => {
  const dir = mkdtempSync(join(process.cwd(), "test/lint-fixture-"));
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

test("business UI uses shared primitives instead of raw controls", () => {
  const raw =
    /<(button|input|select|textarea|label|table|thead|tbody|tr|th|td|dialog|details|summary|progress|hr)(?=[\s/>])/g;
  assert.match('<button type="button">', raw);
  assert.doesNotMatch('<Button type="button">', raw);
  const scan = (directory: string) => {
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      const path = join(directory, entry.name);
      if (entry.isDirectory()) {
        if (entry.name !== "ui") scan(path);
      } else if (entry.name.endsWith(".tsx")) {
        assert.doesNotMatch(readFileSync(path, "utf8"), raw, path);
      }
    }
  };
  scan(join(process.cwd(), "src"));
});

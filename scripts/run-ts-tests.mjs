// Discover every source-owned unit and registered integration test, including TSX.
import { readdirSync } from "node:fs";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const runtime = process.argv[2];
if (!["tsx", "strip-types"].includes(runtime)) {
  throw new Error("usage: run-ts-tests.mjs tsx|strip-types (from the package root)");
}
const check = spawnSync("python3", [fileURLToPath(new URL("./check-test-layout.py", import.meta.url))], { stdio: "inherit" });
if (check.error) throw check.error;
if (check.status !== 0) process.exit(check.status ?? 1);
const files = [];
function discover(directory) {
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) discover(path);
    else if (entry.isFile() && /\.(test|spec)\.tsx?$/.test(entry.name)) files.push(path);
  }
}
for (const directory of ["src", "test"]) discover(directory);
if (!files.length) throw new Error("no TypeScript test files discovered");
files.sort();
console.log(`Running ${files.length} TypeScript test files`);
const options = runtime === "tsx" ? ["--import", "tsx"] : ["--experimental-strip-types"];
const result = spawnSync(process.execPath, [...options, "--test", ...files], { stdio: "inherit" });
if (result.error) throw result.error;
process.exit(result.status ?? 1);

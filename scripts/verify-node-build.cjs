#!/usr/bin/env node
// Validate the bundled npm refresh before executing the package manager.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const root = process.argv[2] || '/opt/npm';
const lock = JSON.parse(fs.readFileSync(process.argv[3] || '/opt/build-policy/node-build.lock.json', 'utf8'));
const semver = require(path.join(root, 'node_modules/semver'));
assert.equal(lock.schema, 1);
const readPackage = (directory) => JSON.parse(fs.readFileSync(path.join(directory, 'package.json'), 'utf8'));
assert.equal(readPackage(root).version, lock.npm.version);
assert(semver.satisfies(process.versions.node, readPackage(root).engines.node), 'Node does not satisfy npm engines');
const replacements = new Map(lock.bundled_overrides.map((entry) => [entry.name, entry.version]));
for (const [name, version] of replacements) {
  assert.equal(readPackage(path.join(root, 'node_modules', name)).version, version);
}
function verify(directory) {
  const metadata = readPackage(directory);
  for (const [name, version] of replacements) {
    const range = metadata.dependencies?.[name];
    if (range) assert(semver.satisfies(version, range), `${metadata.name} requires ${name} ${range}, not ${version}`);
  }
  const modules = path.join(directory, 'node_modules');
  if (!fs.existsSync(modules)) return;
  for (const entry of fs.readdirSync(modules, { withFileTypes: true })) {
    if (!entry.isDirectory() || entry.name.startsWith('.')) continue;
    const child = path.join(modules, entry.name);
    if (entry.name.startsWith('@')) {
      for (const scope of fs.readdirSync(child)) verify(path.join(child, scope));
    } else verify(child);
  }
}
verify(root);
console.log(`Verified Node ${process.versions.node}, npm ${lock.npm.version} and every bundled override constraint.`);

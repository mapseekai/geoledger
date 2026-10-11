#!/usr/bin/env node
// Rebuild the exact npm compiler source with a patched, checksum-pinned Go SDK.
// This is a development-only artifact; nothing from /opt/compiler enters runtime.
const assert = require('node:assert/strict');
const crypto = require('node:crypto');
const fs = require('node:fs');
const path = require('node:path');
const { execFileSync } = require('node:child_process');
const { Readable, Transform } = require('node:stream');
const { pipeline } = require('node:stream/promises');
const policy = process.env.GL_BUILD_POLICY_DIR || '/opt/build-policy';
const lock = JSON.parse(fs.readFileSync(path.join(policy, 'typescript-build.lock.json'), 'utf8'));
const compiler = '/opt/compiler';
const digest = (file) => crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex');

async function download(record, file) {
  assert.match(record.sha256, /^[a-f0-9]{64}$/);
  assert(record.url.startsWith('https://go.dev/dl/') || record.url.startsWith('https://codeload.github.com/microsoft/typescript-go/tar.gz/'));
  const response = await fetch(record.url, { signal: AbortSignal.timeout(180000) });
  if (!response.ok || !response.body) throw new Error(`Build source download failed: HTTP ${response.status}`);
  let bytes = 0;
  const hash = crypto.createHash('sha256');
  await pipeline(Readable.fromWeb(response.body), new Transform({
    transform(chunk, encoding, callback) {
      bytes += chunk.length;
      if (bytes > (record.bytes || 256 * 1024 * 1024)) return callback(new Error('Build archive exceeds expected bound'));
      hash.update(chunk);
      callback(null, chunk);
    },
  }), fs.createWriteStream(file, { flags: 'wx', mode: 0o600 }));
  if (hash.digest('hex') !== record.sha256 || (record.bytes && bytes !== record.bytes)) {
    throw new Error('Build archive checksum/size mismatch');
  }
}

async function build() {
  assert.equal(process.platform, 'linux');
  const architecture = { x64: 'amd64', arm64: 'arm64' }[process.arch];
  assert(architecture && lock.go_archives[architecture], 'Unsupported compiler architecture');
  assert.equal(lock.schema, 1);
  assert.match(lock.source.commit, /^[a-f0-9]{40}$/);
  assert(lock.source.url.endsWith('/' + lock.source.commit));
  fs.mkdirSync(compiler, { recursive: true });
  const temporary = fs.mkdtempSync('/tmp/geoledger-compiler-');
  try {
    const goArchive = path.join(temporary, 'go.tgz');
    const sourceArchive = path.join(temporary, 'source.tgz');
    await download(lock.go_archives[architecture], goArchive);
    await download(lock.source, sourceArchive);
    fs.mkdirSync(path.join(compiler, 'source'));
    execFileSync('/busybox/tar', ['-xzf', goArchive, '-C', compiler]);
    execFileSync('/busybox/tar', ['-xzf', sourceArchive, '-C', path.join(compiler, 'source'), '--strip-components=1']);
    for (const name of ['go.mod', 'go.sum']) {
      const file = path.join(policy, 'typescript-' + name);
      assert.equal(digest(file), lock.module_files[name], 'Compiler module lock drift');
      fs.copyFileSync(file, path.join(compiler, 'source', name));
    }
    const go = path.join(compiler, 'go/bin/go');
    const env = { ...process.env, CGO_ENABLED: '0', GOTOOLCHAIN: 'local', GOOS: 'linux', GOARCH: architecture,
      GOROOT: path.join(compiler, 'go'), GOPATH: path.join(compiler, 'modules') };
    assert.match(execFileSync(go, ['version'], { env, encoding: 'utf8' }), new RegExp(`go${lock.go_version.replaceAll('.', '\\.')}\\s`));
    execFileSync(go, ['build', '-mod=readonly', '-trimpath', '-buildvcs=false', '-o', path.join(compiler, 'tsc'), './cmd/tsgo'],
      { cwd: path.join(compiler, 'source'), env, stdio: 'inherit', timeout: 300000 });
    assert.equal(execFileSync(path.join(compiler, 'tsc'), ['--version'], { encoding: 'utf8' }).trim(), 'Version ' + lock.version);
    fs.writeFileSync(path.join(compiler, 'source-lock.json'), JSON.stringify(lock, null, 2));
    console.log(`Rebuilt TypeScript ${lock.version} from ${lock.source.commit} with Go ${lock.go_version}.`);
  } finally {
    fs.rmSync(temporary, { recursive: true, force: true });
  }
}

function install(projects) {
  assert(projects.length > 0, 'An explicit package directory is required');
  for (const project of projects) {
    const packageRoot = path.join(project, 'node_modules/@typescript', 'typescript-linux-' + process.arch);
    const metadata = JSON.parse(fs.readFileSync(path.join(packageRoot, 'package.json'), 'utf8'));
    assert.equal(metadata.version, lock.version, 'Refusing to replace another compiler release');
    fs.copyFileSync(path.join(compiler, 'tsc'), path.join(packageRoot, 'lib/tsc'));
    fs.chmodSync(path.join(packageRoot, 'lib/tsc'), 0o755);
  }
}

(async () => {
  if (process.argv[2] === 'build') await build();
  else if (process.argv[2] === 'install') install(process.argv.slice(3));
  else throw new Error('Expected build or install with package directories');
})().catch((error) => { console.error(error.message); process.exitCode = 1; });

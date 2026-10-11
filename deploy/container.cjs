#!/usr/bin/env node
const { spawn, spawnSync } = require('node:child_process');
const path = require('node:path');

async function main() {
  if (process.argv[2] === 'probe') {
    const backend = spawnSync('geoledger-server', ['probe'], { stdio: 'inherit', timeout: 5000 });
    if (backend.error || backend.status !== 0) process.exit(1);
    const response = await fetch(`http://127.0.0.1:${process.env.PORT || 3000}/api/health`, {
      signal: AbortSignal.timeout(2000),
    });
    process.exit(response.ok ? 0 : 1);
  }
  if ((process.env.GL_WEB_SESSION_SECRET || '').length < 32) {
    throw new Error('GL_WEB_SESSION_SECRET must contain at least 32 random characters');
  }
  const graceSeconds = Number(process.env.GL_SHUTDOWN_DRAIN_SECS || 0)
    + Number(process.env.GL_SHUTDOWN_TIMEOUT_SECS || 30) + 5;
  if (!Number.isFinite(graceSeconds) || graceSeconds < 5 || graceSeconds > 86400) {
    throw new Error('Invalid shutdown deadline');
  }
  const children = new Set();
  let stopping = false;
  let timer;
  function stop(code, signal = 'SIGTERM') {
    if (stopping) return;
    stopping = true;
    process.exitCode = code;
    for (const child of children) child.kill(signal);
    // Keep the wrapper alive to reap both services, with a bounded shutdown.
    timer = setTimeout(() => {
      for (const child of children) child.kill('SIGKILL');
    }, graceSeconds * 1000);
    if (!children.size) clearTimeout(timer);
  }
  for (const signal of ['SIGTERM', 'SIGINT']) process.on(signal, () => stop(0, signal));
  for (const [command, args] of [
    ['geoledger-server', process.argv.slice(2)],
    [process.execPath, [path.join(__dirname, 'web/server.js')]],
  ]) {
    const child = spawn(command, args, { stdio: 'inherit' });
    children.add(child);
    child.on('error', () => stop(1));
    child.on('close', (code) => {
      children.delete(child);
      // An unexpected clean exit also leaves an incomplete application.
      if (!stopping) stop(code || 1);
      if (!children.size) clearTimeout(timer);
    });
  }
}
main().catch((error) => { console.error(error.message); process.exitCode = 1; });

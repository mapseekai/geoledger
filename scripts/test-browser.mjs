// Exercise the shipped console's JavaScript without a browser or external services.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { webcrypto } from 'node:crypto';
import test from 'node:test';
import vm from 'node:vm';

const html = await readFile(new URL('../crates/server/web/index.html', import.meta.url), 'utf8');
const script = html.match(/<script>([\s\S]*?)<\/script>/)?.[1];
assert.ok(script, 'console contains its embedded script');

function consoleFixture(reply = '{"ok":true}', status = 200) {
  const elements = new Map();
  const calls = [];
  function element(id) {
    if (!elements.has(id)) {
      elements.set(id, {
        value: id === 'version' || id === 'head' ? '0' : '',
        textContent: '', className: '', disabled: false, handlers: new Map(),
        appendChild(option) { if (!this.value) this.value = option.value; },
        addEventListener(event, fn) { this.handlers.set(event, fn); },
      });
    }
    return elements.get(id);
  }
  const forbiddenStorage = new Proxy({}, {
    get() { throw new Error('credentials must not enter browser storage'); },
    set() { throw new Error('credentials must not enter browser storage'); },
  });
  const context = vm.createContext({
    document: { getElementById: element, createElement: () => ({}) },
    crypto: webcrypto, AbortController, Uint8Array, setTimeout, clearTimeout,
    localStorage: forbiddenStorage, sessionStorage: forbiddenStorage,
    fetch: async (path, options) => {
      calls.push({ path, options });
      return { ok: status >= 200 && status < 300, status, text: async () => typeof reply === 'function' ? reply() : reply };
    },
  });
  vm.runInContext(script, context, { timeout: 1000 });
  element('token').value = 'test-only-' + 'x'.repeat(64);
  return {
    element, calls,
    expire() { vm.runInContext('sessionEpoch++', context); },
    async send(operation, raw) {
      element('operation').value = operation;
      element('request').value = raw;
      await element('send').handlers.get('click')();
    },
  };
}

test('console transmits exact integer tokens rather than Number-roundtripping', async () => {
  const c = consoleFixture();
  const raw = '{"project":"p","workspace":"w","expected_workspace_version":1,"edits":[{"dataset":"d","feature_id":"f","feature":{"type":"Feature","id":"f","properties":{"n":9007199254740993,"nested":[18446744073709551615]},"geometry":null}}]}';
  await c.send('save', raw);
  assert.equal(c.calls.length, 1);
  assert.equal(c.calls[0].options.body, raw);
  assert.equal(c.element('status').textContent, 'HTTP 200');
  assert.equal(c.element('send').disabled, false);
});

test('console renders response numeric tokens exactly', async () => {
  const raw = '{"type":"Feature","id":"f","properties":{"n":9007199254740993},"geometry":null}';
  const c = consoleFixture(raw);
  await c.send('features', '{"project":"p","dataset":"d"}');
  assert.match(c.element('response').textContent, /9007199254740993/);
  assert.doesNotMatch(c.element('response').textContent, /9007199254740992/);
});

test('publication retry preserves request identity and payload', async () => {
  const c = consoleFixture('{"revision":2,"version":3,"status":"published"}');
  const raw = '{"project":"p","workspace":"w","expected_workspace_version":2,"request_id":"8b12b4af-2d12-4cc5-a9c3-39b0b727c133","message":"Retry"}';
  await c.send('publish', raw);
  await c.send('publish', raw);
  assert.equal(c.calls.length, 2);
  assert.equal(c.calls[0].options.body, c.calls[1].options.body);
});

test('malformed JSON never leaves the page', async () => {
  const c = consoleFixture();
  await c.send('save', '{');
  assert.equal(c.calls.length, 0);
});

test('structured errors remain visible and credentials stay out of storage', async () => {
  const raw = '{"error":{"code":"conflict","message":"stale version","request_id":"test"}}';
  const c = consoleFixture(raw, 409);
  await c.send('publish', '{}');
  assert.equal(c.element('status').textContent, 'HTTP 409');
  assert.match(c.element('response').textContent, /request_id/);
  assert.equal(c.calls[0].options.credentials, 'omit');
  assert.equal(c.calls[0].options.cache, 'no-store');
});


test('a response from an ended session cannot restore protected content', async () => {
  let complete;
  const reply = new Promise(resolve => { complete = resolve; });
  const c = consoleFixture(() => reply);
  const pending = c.send('features', '{"project":"p","dataset":"d"}');
  c.expire();
  complete('{"properties":{"private":"old session"}}');
  await pending;
  assert.equal(c.element('response').textContent, '');
  assert.equal(c.element('send').disabled, false);
});

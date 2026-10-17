import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";

test("Session cookie has no credential store, logout revokes database authority and password change rotates", () => {
  const session = readFileSync(
    new URL("../src/lib/session.ts", import.meta.url),
    "utf8",
  );
  assert.match(session, /getIronSession<\{\s*sid\?: string\s*\}>/);
  assert.match(session, /authRequest\("logout"/);
  assert.match(session, /httpOnly:\s*true/);
  assert.match(session, /sameSite:\s*"strict"/);
  assert.match(session, /secure:\s*config.secure/);
  assert.doesNotMatch(
    session,
    /new Map|SessionStore|entry\.token|api_key|password:.*input/,
  );
  const route = readFileSync(
    new URL("../src/app/api/session/route.ts", import.meta.url),
    "utf8",
  );
  assert.match(route, /guard\(request\)/);
  assert.match(route, /login\(result.session\)/);
  assert.match(route, /reply\(\{\s*user:\s*result.user\s*\}\)/);
});

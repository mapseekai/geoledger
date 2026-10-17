import assert from "node:assert/strict";
import test from "node:test";
import { configuration } from "./config";

test("deployments require a secret and HTTPS for non-loopback production origins", () => {
  const saved = { ...process.env };
  try {
    delete process.env.GL_WEB_SESSION_SECRET;
    assert.throws(configuration, /SECRET/);
    process.env.GL_WEB_SESSION_SECRET = "test-only-secret-".repeat(3);
    Object.assign(process.env, { NODE_ENV: "production" });
    process.env.GL_WEB_ORIGIN = "http://console.example.com";
    assert.throws(configuration, /HTTPS/);
    process.env.GL_WEB_ORIGIN = "https://console.example.com";
    assert.equal(configuration().secure, true);
    process.env.GL_WEB_ORIGIN = "http://127.0.0.1:3000";
    assert.equal(configuration().secure, false);
  } finally {
    for (const name of ["GL_WEB_SESSION_SECRET", "NODE_ENV", "GL_WEB_ORIGIN"]) {
      if (saved[name] === undefined) delete process.env[name];
      else process.env[name] = saved[name];
    }
  }
});

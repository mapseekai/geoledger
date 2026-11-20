import assert from "node:assert/strict";
import test from "node:test";
import { authRequestSchema } from "./auth-requests";

const current_password = "current-test-password";
const new_password = "new-test-password";

test("each current auth action has a closed request shape", () => {
  const requests: Record<string, unknown>[] = [
    { action: "me" },
    { action: "change_password", current_password, new_password },
    {
      action: "create_user",
      current_password,
      username: "alice",
      password: new_password,
    },
    { action: "list_users" },
    { action: "update_user", current_password, user_id: "user" },
    {
      action: "reset_password",
      current_password,
      user_id: "user",
      new_password,
    },
    { action: "create_api_key", current_password, name: "automation" },
    { action: "list_api_keys" },
    { action: "update_api_key", current_password, key_id: "key" },
    { action: "revoke_api_key", current_password, key_id: "key" },
    { action: "delete_api_key", current_password, key_id: "key" },
  ];
  for (const input of requests) {
    assert(authRequestSchema.safeParse(input).success, String(input.action));
    assert.equal(
      authRequestSchema.safeParse({ ...input, extra: true }).success,
      false,
    );
    for (const field of Object.keys(input)) {
      const missing = { ...input };
      delete missing[field];
      assert.equal(
        authRequestSchema.safeParse(missing).success,
        false,
        `${input.action}: ${field}`,
      );
      assert.equal(
        authRequestSchema.safeParse({ ...input, [field]: null }).success,
        false,
        `${input.action}: ${field}`,
      );
    }
  }
  for (const action of [
    "login_password",
    "login_api_key",
    "logout",
    "issue_token",
  ])
    assert.equal(authRequestSchema.safeParse({ action }).success, false);
});

test("auth preserves omitted versus null expiry and rejects loose optional types", () => {
  const input = { action: "update_api_key", current_password, key_id: "key" };
  const omitted = authRequestSchema.parse(input);
  assert.equal(Object.hasOwn(omitted, "expires_at"), false);
  for (const expires_at of [null, "2099-01-01T00:00:00Z"]) {
    const parsed = authRequestSchema.parse({ ...input, expires_at });
    assert.equal(Object.hasOwn(parsed, "expires_at"), true);
    assert.equal((parsed as Record<string, unknown>).expires_at, expires_at);
  }
  for (const invalid of [
    { ...input, name: null },
    { ...input, expires_at: 7 },
    {
      action: "update_user",
      current_password,
      user_id: "user",
      is_admin: "false",
    },
    {
      action: "update_user",
      current_password,
      user_id: "user",
      display_name: null,
    },
    { action: "list_users", current_password },
    { action: "list_users", limit: null },
    { action: "list_api_keys", limit: 101 },
    { action: "me", current_password },
  ])
    assert.equal(authRequestSchema.safeParse(invalid).success, false);
  assert(
    authRequestSchema.safeParse({
      action: "list_users",
      after: "",
      limit: 50,
      status: "",
    }).success,
  );
});

test("sensitive auth requests retain the password byte limits", () => {
  const input = {
    action: "change_password",
    current_password: "😀".repeat(256),
    new_password,
  };
  assert(authRequestSchema.safeParse(input).success);
  assert.equal(
    authRequestSchema.safeParse({
      ...input,
      current_password: input.current_password + "a",
    }).success,
    false,
  );
  assert.equal(
    authRequestSchema.safeParse({ ...input, new_password: "short" }).success,
    false,
  );
});

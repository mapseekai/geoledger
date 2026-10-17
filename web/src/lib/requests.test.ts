import assert from "node:assert/strict";
import test from "node:test";
import { requestSchema } from "./requests";

test("member sensitive-operation boundary enforces current password bytes", () => {
  const input = {
    action: "removeMember",
    project: "p",
    subject: "u",
    currentPassword: "😀".repeat(256),
  };
  assert.equal(requestSchema.safeParse(input).success, true);
  assert.equal(
    requestSchema.safeParse({
      ...input,
      currentPassword: input.currentPassword + "a",
    }).success,
    false,
  );
});

test("invalid actions, extra upstream addresses, missing delete values and unsafe revisions fail closed", () => {
  for (const value of [
    { action: "unknown" },
    { action: "info", endpoint: "http://attacker" },
    {
      action: "save",
      project: "p",
      workspace: "w",
      version: "1",
      edits: [{ dataset: "d", featureId: "f" }],
    },
    {
      action: "publish",
      project: "p",
      workspace: "w",
      version: 9007199254740992,
      requestId: crypto.randomUUID(),
      message: "m",
    },
    { action: "history", project: "p", after: "9223372036854775808" },
    { action: "setMember", project: "p", subject: "s", role: "none" },
  ])
    assert.equal(requestSchema.safeParse(value).success, false);
  assert(
    requestSchema.safeParse({
      action: "save",
      project: "p",
      workspace: "w",
      version: "1",
      edits: [{ dataset: "d", featureId: "f", feature: null }],
    }).success,
  );
});

test("membership and lifecycle requests are validated", () => {
  for (const value of [
    { action: "members", project: "p", limit: 20 },
    {
      action: "removeMember",
      project: "p",
      subject: "alice",
      currentPassword: "test-only-password",
    },
    { action: "archiveProject", project: "p", archived: true },
    { action: "deleteProject", project: "p", confirmName: "roads" },
  ])
    assert(requestSchema.safeParse(value).success, JSON.stringify(value));
  for (const value of [
    { action: "members", project: "p", limit: 1000 },
    { action: "removeMember", project: "p" },
    { action: "archiveProject", project: "p", archived: "yes" },
    { action: "deleteProject", project: "p", confirmName: "" },
    { action: "deleteProject", project: "p" },
  ])
    assert.equal(requestSchema.safeParse(value).success, false);
});

test("existing 256-byte feature identifiers remain editable through the web contract", () => {
  const edit = { dataset: "d", featureId: "a".repeat(256), feature: null };
  assert(
    requestSchema.safeParse({
      action: "save",
      project: "p",
      workspace: "w",
      version: "1",
      edits: [edit],
    }).success,
  );
  edit.featureId += "b";
  assert.equal(
    requestSchema.safeParse({
      action: "save",
      project: "p",
      workspace: "w",
      version: "1",
      edits: [edit],
    }).success,
    false,
  );
});

test("legacy token management actions are rejected", () => {
  for (const action of ["tokens", "issueToken", "reissueToken", "revokeToken"])
    assert.equal(requestSchema.safeParse({ action }).success, false);
});

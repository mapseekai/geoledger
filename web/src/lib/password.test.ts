import test from "node:test";
import assert from "node:assert/strict";
import {
  passwordError,
  validatePasswordForm,
  type PasswordForm,
} from "./password";

test("password bounds count UTF-8 bytes for ASCII, CJK and emoji", () => {
  for (const value of [
    "a".repeat(12),
    "甲乙丙丁",
    "😀😀😀",
    "甲".repeat(341) + "a",
    "😀".repeat(256),
  ]) {
    assert.equal(passwordError(value), "");
  }
  for (const value of [
    "",
    "a".repeat(11),
    "甲乙丙",
    "😀😀",
    "a".repeat(1025),
    "甲".repeat(342),
    "😀".repeat(256) + "a",
  ]) {
    assert.match(passwordError(value), /UTF-8 12–1024 字节/);
  }
  assert.equal(passwordError("甲", 1), "");
  assert.match(passwordError("", 1), /1–1024/);
  assert.match(passwordError("😀".repeat(257), 1), /1–1024/);
});

test("submit revalidates autofilled current, new and confirmation fields", () => {
  const inputs = [1, 12, 12].map((minimum) => ({
    value: "😀".repeat(257),
    dataset: { passwordMin: String(minimum) },
    validationMessage: "",
    setCustomValidity(message: string) {
      this.validationMessage = message;
    },
  }));
  const form = {
    querySelectorAll: () => inputs,
    reportValidity: () => inputs.every((input) => !input.validationMessage),
  } satisfies PasswordForm;
  assert.equal(validatePasswordForm(form), false);
  inputs.forEach((input) => {
    input.value = "甲乙丙丁";
  });
  assert.equal(validatePasswordForm(form), true);
});

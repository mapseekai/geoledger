export function passwordError(value: string, minimum = 12): string {
  const bytes = new TextEncoder().encode(value).length;
  return bytes < minimum || bytes > 1024
    ? `密码必须为 UTF-8 ${minimum}–1024 字节。`
    : "";
}

export function validatePasswordForm(form: HTMLFormElement): boolean {
  for (const input of form.querySelectorAll<HTMLInputElement>(
    "input[data-password-min]",
  )) {
    input.setCustomValidity(
      passwordError(input.value, Number(input.dataset.passwordMin)),
    );
  }
  return form.reportValidity();
}

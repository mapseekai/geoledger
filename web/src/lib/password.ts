export function passwordError(value: string, minimum = 12): string {
  const bytes = new TextEncoder().encode(value).length;
  return bytes < minimum || bytes > 1024
    ? `密码必须为 UTF-8 ${minimum}–1024 字节。`
    : "";
}

type PasswordInput = Pick<
  HTMLInputElement,
  "value" | "dataset" | "setCustomValidity"
>;
export interface PasswordForm {
  querySelectorAll(selector: string): Iterable<PasswordInput>;
  reportValidity(): boolean;
}

export function validatePasswordForm(form: PasswordForm): boolean {
  for (const input of form.querySelectorAll("input[data-password-min]")) {
    input.setCustomValidity(
      passwordError(input.value, Number(input.dataset.passwordMin)),
    );
  }
  return form.reportValidity();
}

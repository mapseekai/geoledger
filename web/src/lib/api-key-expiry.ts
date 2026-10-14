/** datetime-local is shown in UTC; preserve the stored instant on a name-only edit. */
export function keyExpiryInput(expiresAt?: string | null): string {
  return expiresAt ? new Date(expiresAt).toISOString().slice(0, 19) : "";
}
export function keyExpiryChange(
  original: string | null | undefined,
  input: string,
): string | null | undefined {
  if (original !== undefined && input === keyExpiryInput(original))
    return undefined;
  return input ? new Date(`${input}Z`).toISOString() : null;
}

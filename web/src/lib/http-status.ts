/** Admission failures cannot settle the result of an earlier publication attempt. */
export function uncertainStatus(status: number): boolean {
  return status === 0 || [408, 413, 429, 499].includes(status) || status >= 500;
}
export function httpCode(status: number): string {
  const codes: Record<number, string> = {
    400: "invalid_argument",
    401: "unauthenticated",
    403: "permission_denied",
    404: "not_found",
    408: "deadline_exceeded",
    409: "conflict",
    413: "resource_exhausted",
    422: "invalid_argument",
    429: "resource_exhausted",
    499: "cancelled",
    504: "deadline_exceeded",
  };
  return codes[status] ?? "unavailable";
}

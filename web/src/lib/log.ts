import "server-only";
/** One JSON line per server-side console failure, without tokens, cookies or request bodies. */
export function logFailure(entry: {
  status: number;
  code?: string;
  requestId?: string;
  error?: unknown;
}) {
  const error =
    entry.error instanceof Error
      ? {
          name: entry.error.name,
          message: entry.error.message.slice(0, 300),
        }
      : undefined;
  console.error(
    JSON.stringify({
      level: entry.status >= 500 ? "error" : "warn",
      time: new Date().toISOString(),
      msg: "console request failed",
      status: entry.status,
      code: entry.code,
      requestId: entry.requestId,
      error,
    }),
  );
}

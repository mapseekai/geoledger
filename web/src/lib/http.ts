import "server-only";
import { GeoLedgerError, stringifyJson } from "@geoledger/client";
import { ZodError } from "zod";
import { configuration } from "./config";
import { encode } from "./operations";
import { logFailure } from "./log";
export class HttpError extends Error {
  constructor(
    public status: number,
    message: string,
  ) {
    super(message);
  }
}
export function guard(request: Request) {
  if (
    request.headers.get("origin") !== configuration().origin ||
    request.headers.get("sec-fetch-site") === "cross-site"
  )
    throw new HttpError(403, "请求来源不匹配，请通过配置的控制台地址访问。");
  if (
    request.headers.get("content-type")?.split(";")[0].trim() !==
    "application/json"
  )
    throw new HttpError(415, "需要 JSON 请求。");
}
export async function body(
  request: Request,
  maxBytes = Infinity,
): Promise<unknown> {
  if (Number(request.headers.get("content-length")) > maxBytes)
    throw new HttpError(413, "请求过大。");
  const reader = request.body?.getReader();
  if (!reader) throw new HttpError(400, "请求内容为空。");
  let timedOut = false;
  const timeout = setTimeout(() => {
    timedOut = true;
    void reader.cancel();
  }, 15_000);
  const chunks: Uint8Array[] = [];
  let length = 0;
  try {
    while (true) {
      const { value, done } = await reader.read();
      if (timedOut) throw new HttpError(408, "请求读取超时。");
      if (done) break;
      length += value.length;
      if (length > maxBytes) {
        await reader.cancel();
        throw new HttpError(413, "请求过大。");
      }
      chunks.push(value);
    }
    return JSON.parse(Buffer.concat(chunks).toString("utf8"));
  } catch (error) {
    if (error instanceof HttpError) throw error;
    throw new HttpError(400, "JSON 格式无效。");
  } finally {
    clearTimeout(timeout);
    reader.releaseLock();
  }
}
export function reply(data: unknown, status = 200) {
  return new Response(encode(data), {
    status,
    headers: {
      "content-type": "application/json; charset=utf-8",
      "cache-control": "no-store",
    },
  });
}
export function geoReply(data: unknown) {
  return new Response(stringifyJson(data), {
    headers: {
      "content-type": "application/geo+json; charset=utf-8",
      "cache-control": "private, no-store",
      "x-content-type-options": "nosniff",
    },
  });
}
export function guardRead(request: Request) {
  const origin = request.headers.get("origin");
  if (
    (origin && origin !== configuration().origin) ||
    request.headers.get("sec-fetch-site") === "cross-site"
  )
    throw new HttpError(403, "请求来源不匹配。");
}
export function failure(error: unknown) {
  if (error instanceof HttpError)
    return reply({ error: { message: error.message } }, error.status);
  if (error instanceof ZodError)
    return reply(
      {
        error: {
          message: "参数格式不正确，请检查名称、版本与输入内容。",
          code: "invalid_argument",
        },
      },
      400,
    );
  if (error instanceof GeoLedgerError) {
    const statuses: Record<string, number> = {
      unauthenticated: 401,
      permission_denied: 403,
      forbidden: 403,
      not_found: 404,
      conflict: 409,
      invalid_argument: 400,
      bad_request: 400,
      resource_exhausted: 429,
      timeout: 504,
    };
    const status = statuses[error.code] ?? 503;
    if (status >= 500)
      logFailure({ status, code: error.code, requestId: error.requestId });
    return reply(
      {
        error: {
          code: error.code,
          message: error.message,
          requestId: error.requestId,
          uncertain: error.uncertain,
        },
      },
      status,
    );
  }
  logFailure({ status: 503, error });
  // Avoid leaking cookies, credentials or upstream connection diagnostics into logs/responses.
  return reply(
    {
      error: {
        message: "控制台服务暂时不可用，请检查服务配置。",
        uncertain: true,
      },
    },
    503,
  );
}

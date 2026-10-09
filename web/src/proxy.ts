import { NextResponse, type NextRequest } from "next/server";
import { rasterBasemap } from "@/lib/basemap";
export function proxy(request: NextRequest) {
  const nonce = Buffer.from(crypto.randomUUID()).toString("base64");
  const dev = process.env.NODE_ENV !== "production";
  // Only an operator-configured raster basemap origin is added to the policy.
  const tiles = rasterBasemap(process.env)?.origin;
  const extra = tiles ? ` ${tiles}` : "";
  const csp = `default-src 'self'; script-src 'self' 'nonce-${nonce}' 'strict-dynamic'${dev ? " 'unsafe-eval'" : ""}; style-src 'self' 'unsafe-inline'; worker-src 'self'; img-src 'self' data: blob:${extra}; font-src 'self'; connect-src 'self'${extra}; object-src 'none'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'`;
  const headers = new Headers(request.headers);
  headers.set("x-nonce", nonce);
  headers.set("Content-Security-Policy", csp);
  const response = NextResponse.next({ request: { headers } });
  response.headers.set("Content-Security-Policy", csp);
  response.headers.set("Cache-Control", "private, no-store");
  return response;
}
export const config = {
  // JSON APIs validate sessions and Origin in route handlers; skip page CSP
  // middleware so Next.js does not buffer/truncate large API request bodies.
  matcher: ["/((?!api/|_next/static|_next/image|logo.svg|favicon.ico).*)"],
};

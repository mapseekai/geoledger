export function configuration() {
  const origin = new URL(process.env.GL_WEB_ORIGIN ?? "http://localhost:3000");
  if (
    !["http:", "https:"].includes(origin.protocol) ||
    origin.href !== `${origin.origin}/`
  )
    throw new Error("GL_WEB_ORIGIN must be an HTTP(S) origin");
  const password = process.env.GL_WEB_SESSION_SECRET;
  if (!password || password.length < 32)
    throw new Error(
      "GL_WEB_SESSION_SECRET must contain at least 32 random characters",
    );
  if (
    process.env.NODE_ENV === "production" &&
    origin.protocol !== "https:" &&
    !["localhost", "127.0.0.1", "[::1]"].includes(origin.hostname)
  )
    throw new Error(
      "Production GL_WEB_ORIGIN requires HTTPS (except loopback)",
    );
  return {
    origin: origin.origin,
    secure: origin.protocol === "https:",
    password,
    endpoint: process.env.GL_WEB_ENDPOINT ?? "http://127.0.0.1:7882",
  };
}

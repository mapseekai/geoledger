import { uncertainStatus } from "./http-status";
export type Publication = Readonly<{
  action: "publish";
  project: string;
  workspace: string;
  version: string;
  requestId: string;
  message: string;
}>;
export function publication(
  project: string,
  workspace: string,
  version: string,
  message: string,
): Publication {
  return Object.freeze({
    action: "publish",
    project,
    workspace,
    version,
    message,
    requestId: crypto.randomUUID(),
  });
}
export function readPublication(raw: string | null): Publication | undefined {
  if (!raw) return;
  try {
    const p = JSON.parse(raw);
    if (
      p.action === "publish" &&
      [p.project, p.workspace, p.version, p.requestId, p.message].every(
        (v) => typeof v === "string" && v.length > 0,
      )
    )
      return Object.freeze(p) as Publication;
  } catch {
    /* A damaged client-side intent never becomes a new publication automatically. */
  }
}

/** A rejection of one attempt cannot settle a previously unknown publication. */
export function releasePublication(
  status: number,
  uncertain: boolean,
  wasPending: boolean,
  code?: string,
): boolean {
  if (code === "publication_attempt_rejected") return !wasPending;
  return (
    !uncertain &&
    (!wasPending ||
      (!uncertainStatus(status) &&
        ![400, 401, 403, 404].includes(status) &&
        code !== "invalid_argument"))
  );
}

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

/** The engine issues this permanent proof only for an exhausted workspace version. */
export function invalidVersionProof(
  status: number,
  code: unknown,
  message: unknown,
  requestId: unknown,
  conflicts: unknown,
): boolean {
  return (
    code === "workspace_version_conflict" &&
    (status !== 409 ||
      typeof message !== "string" ||
      !message.trim() ||
      typeof requestId !== "string" ||
      !requestId.trim() ||
      conflicts !== undefined)
  );
}

/** Engine integers are decoded to bigint; the browser transport uses decimal strings. */
export function matchesPublicationReceipt(
  value: unknown,
  workspace: unknown,
  expected: unknown,
): boolean {
  if (
    !value ||
    typeof value !== "object" ||
    typeof workspace !== "string" ||
    !workspace
  )
    return false;
  const receipt = value as Record<string, unknown>;
  if (receipt.workspace !== workspace || receipt.status !== "published")
    return false;
  const integer = (field: unknown): bigint | undefined => {
    if (typeof expected === "bigint")
      return typeof field === "bigint" ? field : undefined;
    return typeof field === "string" && /^(0|[1-9][0-9]{0,19})$/.test(field)
      ? BigInt(field)
      : undefined;
  };
  const previous = integer(expected);
  const revision = integer(receipt.revision);
  const version = integer(receipt.version);
  const changes = integer(receipt.changes);
  const maxRevision = (1n << 63n) - 1n;
  return (
    previous !== undefined &&
    previous >= 0n &&
    version !== undefined &&
    version <= maxRevision &&
    version === previous + 1n &&
    revision !== undefined &&
    revision > 0n &&
    revision <= maxRevision &&
    changes !== undefined &&
    changes >= 0n &&
    changes <= (1n << 64n) - 1n
  );
}

/** A rejection of one attempt cannot settle a previously unknown publication. */
export function releasePublication(
  status: number,
  uncertain: boolean,
  wasPending: boolean,
  code?: string,
): boolean {
  return (
    !uncertain &&
    (!wasPending || (status === 409 && code === "workspace_version_conflict"))
  );
}

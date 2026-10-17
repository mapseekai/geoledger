import type { ConsoleClient } from "../../src/lib/console-client";

/** Unexpected calls fail locally; overrides must implement their real method contract. */
export function stubConsoleClient(
  overrides: Partial<ConsoleClient>,
): ConsoleClient {
  const unexpected = (): never => {
    throw new Error("unexpected console operation");
  };
  return {
    info: unexpected,
    projects: unexpected,
    project: unexpected,
    createProject: unexpected,
    datasets: unexpected,
    renameProject: unexpected,
    renameDataset: unexpected,
    deleteDataset: unexpected,
    createDataset: unexpected,
    trackTable: unexpected,
    workspaces: unexpected,
    workspaceInfo: unexpected,
    workspaceSummary: unexpected,
    commitSummary: unexpected,
    createWorkspace: unexpected,
    features: unexpected,
    save: unexpected,
    publish: unexpected,
    diff: unexpected,
    conflicts: unexpected,
    resolve: unexpected,
    rebase: unexpected,
    discard: unexpected,
    history: unexpected,
    commit: unexpected,
    restore: unexpected,
    audit: unexpected,
    members: unexpected,
    archiveProject: unexpected,
    deleteProject: unexpected,
    setMember: unexpected,
    removeMember: unexpected,
    ...overrides,
  };
}

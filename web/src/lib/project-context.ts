import { ApiError, type Project } from "./browser-api";

export const currentProjectKey = "gl.currentProject";

type ProjectResolution = {
  project?: Project;
  source: "explicit" | "remembered" | "default" | "empty";
  forgotRemembered: boolean;
};

type ProjectResolver = {
  explicit: string | undefined;
  remembered: string | null;
  getProject: (id: string) => Promise<Project>;
  listProjectIds: () => Promise<string[]>;
};

export async function resolveProject({
  explicit,
  remembered,
  getProject,
  listProjectIds,
}: ProjectResolver): Promise<ProjectResolution> {
  if (explicit !== undefined) {
    return {
      project: await getProject(explicit),
      source: "explicit",
      forgotRemembered: false,
    };
  }
  if (remembered) {
    try {
      return {
        project: await getProject(remembered),
        source: "remembered",
        forgotRemembered: false,
      };
    } catch (error) {
      if (!(error instanceof ApiError) || ![403, 404].includes(error.status))
        throw error;
    }
  }
  const id = (await listProjectIds())[0];
  if (!id) return { source: "empty", forgotRemembered: Boolean(remembered) };
  return {
    project: await getProject(id),
    source: "default",
    forgotRemembered: Boolean(remembered),
  };
}

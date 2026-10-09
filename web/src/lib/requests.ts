import { z } from "zod";
const text = (bytes: number) =>
  z
    .string()
    .min(1)
    .refine((value) => new TextEncoder().encode(value).length <= bytes);
const id = text(200);
const featureId = text(256);
const revision = z
  .string()
  .regex(/^(0|[1-9][0-9]*)$/)
  .refine((v) => BigInt(v) <= 9223372036854775807n);
const page = {
  after: z.string().max(512).optional(),
  limit: z.number().int().min(1).max(100).optional(),
};
const project = { project: id };
const workspace = { ...project, workspace: id };
const version = { ...workspace, version: revision };
const edit = z.strictObject({
  dataset: id,
  featureId,
  feature: z.string().min(1).nullable(),
});
const variants = {
  info: {},
  projects: page,
  project,
  createProject: { name: text(256) },
  renameProject: { ...project, name: text(256) },
  renameDataset: { ...project, dataset: id, name: text(256) },
  deleteDataset: { ...project, dataset: id, confirmName: text(256) },
  datasets: { ...project, ...page },
  createDataset: {
    ...project,
    name: text(256),
    geometryType: z.enum(["point", "line", "polygon"]),
  },
  workspaces: { ...project, ...page },
  createWorkspace: project,
  workspace,
  workspaceSummary: workspace,
  commitSummary: { ...project, revision },
  features: {
    ...project,
    dataset: id,
    ...page,
    workspace: id.optional(),
    revision: revision.optional(),
    featureId: featureId.optional(),
  },
  save: { ...version, edits: z.array(edit).min(1).max(100) },
  publish: { ...version, requestId: z.uuid(), message: text(2048) },
  diff: { ...workspace, ...page },
  conflicts: { ...workspace, ...page },
  resolve: { ...version, head: revision, edits: z.array(edit).max(100) },
  rebase: { ...version, head: revision, edits: z.array(edit).max(100) },
  discard: version,
  history: { ...project, after: revision.optional(), limit: page.limit },
  commit: { ...project, revision, ...page },
  restore: { ...project, revision },
  audit: { ...project, after: revision.optional(), limit: page.limit },
  setMember: {
    ...project,
    subject: text(128),
    role: z.enum(["owner", "editor", "viewer"]),
  },
  members: { ...project, ...page },
  removeMember: { ...project, subject: text(128) },
  archiveProject: { ...project, archived: z.boolean() },
  deleteProject: { ...project, confirmName: text(256) },
};
export const requestSchema = z.discriminatedUnion("action", [
  z.strictObject({
    action: z.literal("renameProject"),
    ...variants.renameProject,
  }),
  z.strictObject({
    action: z.literal("renameDataset"),
    ...variants.renameDataset,
  }),
  z.strictObject({
    action: z.literal("deleteDataset"),
    ...variants.deleteDataset,
  }),

  z.strictObject({ action: z.literal("info"), ...variants.info }),
  z.strictObject({ action: z.literal("projects"), ...variants.projects }),
  z.strictObject({ action: z.literal("project"), ...variants.project }),
  z.strictObject({
    action: z.literal("createProject"),
    ...variants.createProject,
  }),
  z.strictObject({ action: z.literal("datasets"), ...variants.datasets }),
  z.strictObject({
    action: z.literal("createDataset"),
    ...variants.createDataset,
  }),
  z.strictObject({ action: z.literal("workspaces"), ...variants.workspaces }),
  z.strictObject({
    action: z.literal("createWorkspace"),
    ...variants.createWorkspace,
  }),
  z.strictObject({ action: z.literal("workspace"), ...variants.workspace }),
  z.strictObject({
    action: z.literal("workspaceSummary"),
    ...variants.workspaceSummary,
  }),
  z.strictObject({
    action: z.literal("commitSummary"),
    ...variants.commitSummary,
  }),
  z.strictObject({ action: z.literal("features"), ...variants.features }),
  z.strictObject({ action: z.literal("save"), ...variants.save }),
  z.strictObject({ action: z.literal("publish"), ...variants.publish }),
  z.strictObject({ action: z.literal("diff"), ...variants.diff }),
  z.strictObject({ action: z.literal("conflicts"), ...variants.conflicts }),
  z.strictObject({ action: z.literal("resolve"), ...variants.resolve }),
  z.strictObject({ action: z.literal("rebase"), ...variants.rebase }),
  z.strictObject({ action: z.literal("discard"), ...variants.discard }),
  z.strictObject({ action: z.literal("history"), ...variants.history }),
  z.strictObject({ action: z.literal("commit"), ...variants.commit }),
  z.strictObject({ action: z.literal("restore"), ...variants.restore }),
  z.strictObject({ action: z.literal("audit"), ...variants.audit }),
  z.strictObject({ action: z.literal("setMember"), ...variants.setMember }),
  z.strictObject({ action: z.literal("members"), ...variants.members }),
  z.strictObject({
    action: z.literal("removeMember"),
    ...variants.removeMember,
  }),
  z.strictObject({
    action: z.literal("archiveProject"),
    ...variants.archiveProject,
  }),
  z.strictObject({
    action: z.literal("deleteProject"),
    ...variants.deleteProject,
  }),
]);

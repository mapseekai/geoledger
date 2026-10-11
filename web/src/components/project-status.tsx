"use client";
import { useRef, useState } from "react";
import { Archive, ArchiveRestore, Trash2 } from "lucide-react";
import { call, type Project } from "@/lib/browser-api";
import { ErrorBox, Modal } from "./common";
import { Panel, PanelTitle, useAction } from "./resource-shared";
import { Badge } from "./ui/badge";
import { Button } from "./ui/button";
import { Input } from "./ui/input";
import { Field, FieldGroup, FieldLabel } from "./ui/field";

export function ProjectStatus({ project }: { project: Project }) {
  const task = useAction();
  const [state, setState] = useState(project.state);
  const [deleting, setDeleting] = useState(false);
  const deleteTrigger = useRef<HTMLButtonElement>(null);
  const owner = project.role === "owner" || project.role === "admin";
  const archived = state === "archived";
  return (
    <div className="grid gap-6">
      <Panel toolbar={<PanelTitle title="当前状态" />}>
        <div className="flex flex-wrap items-center justify-between gap-4 p-6">
          <div className="grid gap-2">
            <div>
              <Badge variant={archived ? "secondary" : "outline"}>
                {archived ? "已归档" : "活跃"}
              </Badge>
            </div>
            <p className="text-sm text-muted-foreground">
              {archived
                ? "项目数据与版本已保留，恢复后可继续编辑。"
                : "项目可正常编辑与发布，归档后保留数据与版本。"}
            </p>
          </div>
          <Button
            variant="outline"
            disabled={!owner || task.busy}
            onClick={() =>
              void task.run(async () => {
                const next = await call<Project>({
                  action: "archiveProject",
                  project: project.id,
                  archived: !archived,
                });
                setState(next.state);
              })
            }
          >
            {archived ? (
              <ArchiveRestore data-icon="inline-start" />
            ) : (
              <Archive data-icon="inline-start" />
            )}
            {task.busy ? "正在更新…" : archived ? "恢复项目" : "归档项目"}
          </Button>
        </div>
        <ErrorBox message={task.error} />
      </Panel>
      <Panel toolbar={<PanelTitle title="删除项目" />}>
        <div className="flex flex-wrap items-center justify-between gap-4 p-6">
          <p className="text-sm text-muted-foreground">
            删除后项目从列表移除，操作前需确认项目名称。
          </p>
          <Button
            ref={deleteTrigger}
            variant="destructive"
            disabled={!owner || task.busy}
            onClick={() => setDeleting(true)}
          >
            <Trash2 data-icon="inline-start" />
            删除项目
          </Button>
        </div>
      </Panel>
      {deleting && (
        <DeleteProjectDialog
          project={project}
          close={() => setDeleting(false)}
          restoreFocus={() => deleteTrigger.current?.focus()}
        />
      )}
    </div>
  );
}
function DeleteProjectDialog({
  project,
  close,
  restoreFocus,
}: {
  project: Project;
  close: () => void;
  restoreFocus: () => void;
}) {
  const task = useAction();
  return (
    <Modal
      title="删除项目"
      size="sm"
      close={close}
      busy={task.busy}
      onCloseAutoFocus={(event) => {
        event.preventDefault();
        restoreFocus();
      }}
      footer={
        <>
          <Button
            type="button"
            variant="outline"
            disabled={task.busy}
            onClick={close}
          >
            取消
          </Button>
          <Button
            type="submit"
            form="delete-project"
            variant="destructive"
            disabled={task.busy}
          >
            {task.busy ? "正在删除…" : "删除项目"}
          </Button>
        </>
      }
    >
      <form
        id="delete-project"
        onSubmit={(event) => {
          event.preventDefault();
          const confirmName = String(
            new FormData(event.currentTarget).get("confirm"),
          );
          void task.run(async () => {
            if (confirmName !== project.name)
              throw new Error("请输入完全一致的项目名称。");
            await call({
              action: "deleteProject",
              project: project.id,
              confirmName,
            });
            window.location.assign("/projects");
          });
        }}
      >
        <FieldGroup>
          <Field>
            <FieldLabel htmlFor="confirm-delete">
              删除项目：输入项目名称“{project.name}”确认
            </FieldLabel>
            <Input
              id="confirm-delete"
              name="confirm"
              required
              autoFocus
              maxLength={256}
              disabled={task.busy}
              autoComplete="off"
            />
          </Field>
          <ErrorBox message={task.error} />
        </FieldGroup>
      </form>
    </Modal>
  );
}

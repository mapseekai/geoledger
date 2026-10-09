"use client";
import {
  call,
  type Changes,
  type Commit,
  type Project,
  type Workspace,
} from "@/lib/browser-api";
import {
  ArrowDownToLine,
  CircleCheck,
  FileSearch,
  History,
} from "lucide-react";
import Link from "next/link";
import { useState } from "react";
import { Empty, ErrorBox, Loading, Modal, Notice, usePage } from "./common";
import { Panel, PanelTitle, time, useAction } from "./resource-shared";
import { Badge } from "./ui/badge";
import { Button } from "./ui/button";
import {
  Table,
  TableCaption,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "./ui/table";
import { ChangeSummary } from "./change-summary";
import { Inspector } from "./workspaces";
export function HistoryPanel({
  project,
  writable,
}: {
  project: Project;
  writable: boolean;
}) {
  const [inspect, setInspect] = useState<Commit>(),
    [restore, setRestore] = useState<Commit>(),
    [created, setCreated] = useState<Workspace>();
  const task = useAction();
  const page = usePage<Commit>(async (after) => {
    const rows = await call<Commit[]>({
      action: "history",
      project: project.id,
      after: after || "0",
      limit: 20,
    });
    return {
      rows,
      next: rows.length === 20 ? rows.at(-1)!.revision : undefined,
    };
  }, 0);
  return (
    <>
      <Panel
        toolbar={
          <>
            <PanelTitle title="已发布版本" />
          </>
        }
      >
        <ErrorBox message={page.error} />
        {page.busy ? (
          <Loading />
        ) : page.rows.length ? (
          <Table aria-label="版本历史">
            <TableCaption className="caption-top p-3 text-left font-medium">
              版本历史与数据集变更
            </TableCaption>
            <TableHeader>
              <TableRow>
                <TableHead>版本</TableHead>
                <TableHead>说明</TableHead>
                <TableHead>发布者</TableHead>
                <TableHead>发布时间</TableHead>
                <TableHead>数据集 / 要素变更</TableHead>
                <TableHead className="cell-actions">操作</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {page.rows.map((c) => (
                <TableRow key={c.revision}>
                  <TableCell>
                    <Badge variant="outline" className="revision-tag">
                      r{c.revision}
                    </Badge>
                  </TableCell>
                  <TableCell className="message-cell">{c.message}</TableCell>
                  <TableCell>
                    <span className="subject">
                      <span className="subject-avatar" aria-hidden="true">
                        {c.subject.slice(0, 1).toUpperCase()}
                      </span>
                      {c.subject}
                    </span>
                  </TableCell>
                  <TableCell className="muted">{time(c.createdAt)}</TableCell>
                  <TableCell>
                    <ChangeSummary
                      project={project.id}
                      revision={c.revision}
                      compact
                    />
                  </TableCell>
                  <TableCell className="cell-actions">
                    <div className="row-actions">
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => setInspect(c)}
                      >
                        <FileSearch />
                        详情
                      </Button>
                      <Button
                        variant="ghost"
                        size="sm"
                        disabled={!writable}
                        onClick={() => {
                          setCreated(undefined);
                          setRestore(c);
                        }}
                      >
                        <ArrowDownToLine />
                        撤销
                      </Button>
                    </div>
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        ) : (
          <Empty icon={History} title="还没有发布记录" />
        )}
        {page.footer}
      </Panel>
      {inspect && (
        <Inspector
          title={`版本 r${inspect.revision}`}
          description={inspect.message}
          summary={
            <ChangeSummary project={project.id} revision={inspect.revision} />
          }
          close={() => setInspect(undefined)}
          load={async (after) => {
            const r = await call<Changes>({
              action: "commit",
              project: project.id,
              revision: inspect.revision,
              after,
              limit: 20,
            });
            return {
              rows: r.changes,
              next:
                r.changes.length === 20 ? r.changes.at(-1)!.cursor : undefined,
            };
          }}
        />
      )}
      {restore && (
        <Modal
          title={`撤销版本 r${restore.revision} 的更改`}
          description="创建撤销工作区，仅反向修改此版本的更改；检查并发布后生效。"
          close={task.busy ? () => {} : () => setRestore(undefined)}
        >
          {created ? (
            <Notice tone="success" icon={<CircleCheck aria-hidden="true" />}>
              <div>
                工作区已创建：<span className="mono">{created.id}</span>
                <p>
                  <Link
                    className="text-link"
                    href={`/workspaces?project=${project.id}`}
                  >
                    前往工作区查看
                  </Link>
                </p>
              </div>
            </Notice>
          ) : (
            <>
              <ErrorBox message={task.error} />
              <div className="form-actions">
                <Button
                  variant="outline"
                  disabled={task.busy}
                  onClick={() => setRestore(undefined)}
                >
                  取消
                </Button>
                <Button
                  disabled={task.busy}
                  onClick={() =>
                    void task.run(async () =>
                      setCreated(
                        await call<Workspace>({
                          action: "restore",
                          project: project.id,
                          revision: restore.revision,
                        }),
                      ),
                    )
                  }
                >
                  创建撤销工作区
                </Button>
              </div>
            </>
          )}
        </Modal>
      )}
    </>
  );
}

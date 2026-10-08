"use client";
import {
  call,
  type Changes,
  type Commit,
  type Project,
  type Workspace,
} from "@/lib/browser-api";
import { ArrowDownToLine } from "lucide-react";
import Link from "next/link";
import { useState } from "react";
import { Empty, ErrorBox, Loading, Modal, usePage } from "./common";
import { Panel, time, useAction } from "./resource-shared";
import { Badge } from "./ui/badge";
import { Button } from "./ui/button";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "./ui/table";
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
            <strong>已发布版本</strong>
            <Badge variant="secondary">可追溯 · 可恢复</Badge>
          </>
        }
      >
        <ErrorBox message={page.error} />
        {page.busy ? (
          <Loading />
        ) : page.rows.length ? (
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>版本</TableHead>
                <TableHead>说明</TableHead>
                <TableHead>发布者</TableHead>
                <TableHead>发布时间</TableHead>
                <TableHead>操作</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {page.rows.map((c) => (
                <TableRow key={c.revision}>
                  <TableCell>
                    <Badge variant="secondary">r{c.revision}</Badge>
                  </TableCell>
                  <TableCell className="message-cell">{c.message}</TableCell>
                  <TableCell>{c.subject}</TableCell>
                  <TableCell className="muted">{time(c.createdAt)}</TableCell>
                  <TableCell>
                    <div className="toolbar-actions">
                      <Button variant="ghost" onClick={() => setInspect(c)}>
                        详情
                      </Button>
                      <Button
                        variant="ghost"
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
          <Empty title="还没有发布记录">工作区发布后，版本会出现在这里。</Empty>
        )}
        {page.footer}
      </Panel>
      {inspect && (
        <Inspector
          title={`版本 r${inspect.revision}`}
          description={inspect.message}
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
          description="创建一个反向编辑工作区，仅撤销此版本涉及的更改，保留其他版本的变化。检查并发布后生效；这不是恢复整个历史快照。"
          close={task.busy ? () => {} : () => setRestore(undefined)}
        >
          {created ? (
            <div className="notice">
              工作区已创建：<span className="mono">{created.id}</span>
              <p>
                <Link href={`/workspaces?project=${project.id}`}>
                  前往工作区查看
                </Link>
              </p>
            </div>
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

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
import { useRouter, useSearchParams } from "next/navigation";
import { useEffect, useState } from "react";
import { toast } from "sonner";
import {
  CopyButton,
  Empty,
  ErrorBox,
  Loading,
  Modal,
  Notice,
  RelativeTime,
  usePage,
} from "./common";
import { Panel, PanelTitle, short, useAction } from "./resource-shared";
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
  const router = useRouter();
  const params = useSearchParams();
  const linked = params.get("revision");
  // `?revision=N` (from the publish dialog) opens that version directly.
  useEffect(() => {
    if (!linked || !/^[0-9]+$/.test(linked)) return;
    let active = true;
    call<Commit[]>({
      action: "history",
      project: project.id,
      after: String(BigInt(linked) - 1n),
      limit: 1,
    })
      .then((rows) => {
        if (active && rows[0]?.revision === linked) setInspect(rows[0]);
      })
      .catch(() => undefined);
    return () => {
      active = false;
    };
  }, [linked, project.id]);
  const closeInspect = () => {
    setInspect(undefined);
    if (linked) {
      const next = new URLSearchParams(params.toString());
      next.delete("revision");
      router.replace(`/history?${next}`);
    }
  };
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
            <TableCaption className="sr-only">
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
                    <span className="rev-cell">
                      <Badge variant="outline" className="revision-tag">
                        r{c.revision}
                      </Badge>
                      {c.revision === project.head && (
                        <Badge className="head-badge">HEAD</Badge>
                      )}
                    </span>
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
                  <TableCell className="muted">
                    <RelativeTime value={c.createdAt} />
                  </TableCell>
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
          project={project.id}
          title={`版本 r${inspect.revision}`}
          description={inspect.message}
          meta={
            <>
              <Badge variant="outline" className="meta-chip">
                {inspect.subject}
              </Badge>
              <Badge variant="outline" className="meta-chip">
                <RelativeTime value={inspect.createdAt} plain />
              </Badge>
              <Badge variant="outline" className="meta-chip">
                <span className="mono">{short(inspect.sourceWorkspace)}</span>
                <CopyButton
                  value={inspect.sourceWorkspace}
                  label="复制来源工作区"
                />
              </Badge>
            </>
          }
          summary={
            <ChangeSummary project={project.id} revision={inspect.revision} />
          }
          close={closeInspect}
          load={async (after) => {
            const r = await call<Changes>({
              action: "commit",
              project: project.id,
              revision: inspect.revision,
              after,
              limit: 50,
            });
            return {
              rows: r.changes,
              next:
                r.changes.length === 50 ? r.changes.at(-1)!.cursor : undefined,
            };
          }}
        />
      )}
      {restore && (
        <Modal
          title={`撤销版本 r${restore.revision} 的更改`}
          description={restore.message}
          size="sm"
          busy={task.busy}
          close={() => setRestore(undefined)}
          footer={
            created ? (
              <>
                <Button variant="outline" onClick={() => setRestore(undefined)}>
                  关闭
                </Button>
                <Button asChild>
                  <Link href={`/workspaces?project=${project.id}`}>
                    前往工作区查看
                  </Link>
                </Button>
              </>
            ) : (
              <>
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
                    void task.run(async () => {
                      const workspace = await call<Workspace>({
                        action: "restore",
                        project: project.id,
                        revision: restore.revision,
                      });
                      setCreated(workspace);
                      toast.success("已创建撤销工作区", {
                        description: workspace.id,
                      });
                    })
                  }
                >
                  创建撤销工作区
                </Button>
              </>
            )
          }
        >
          {created ? (
            <Notice tone="success" icon={<CircleCheck aria-hidden="true" />}>
              <span>
                工作区已创建：<span className="mono">{created.id}</span>
              </span>
              <CopyButton value={created.id} label="复制标识" />
            </Notice>
          ) : (
            <>
              <ChangeSummary project={project.id} revision={restore.revision} />
              <ErrorBox message={task.error} />
            </>
          )}
        </Modal>
      )}
    </>
  );
}

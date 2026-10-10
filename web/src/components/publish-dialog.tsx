"use client";
import {
  ApiError,
  call,
  type Changes,
  type Conflicts,
  type Project,
  type Workspace,
} from "@/lib/browser-api";
import type { ChangeSummary } from "@/lib/change-summary";
import { count } from "@/lib/format";
import {
  publication,
  readPublication,
  releasePublication,
} from "@/lib/publication";
import {
  ArrowRight,
  Check,
  ChevronRight,
  ExternalLink,
  GitMerge,
  RefreshCw,
  TriangleAlert,
  Upload,
} from "lucide-react";
import Link from "next/link";
import { useCallback, useEffect, useState } from "react";
import { toast } from "sonner";
import { ChangeList } from "./change-list";
import {
  CopyButton,
  DiffStat,
  ErrorBox,
  FooterStart,
  Modal,
  Notice,
} from "./common";
import { Resolution } from "./conflict-resolution";
import { short, useAction } from "./resource-shared";
import { Badge } from "./ui/badge";
import { Button } from "./ui/button";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "./ui/collapsible";
import { Label } from "./ui/label";
import { Skeleton } from "./ui/skeleton";
import { Textarea } from "./ui/textarea";

type Preflight = {
  workspace?: Workspace;
  head?: string;
  summary?: ChangeSummary;
  conflicts?: number;
};

export function PublishDialog({
  project,
  workspace,
  version,
  close,
  complete,
}: {
  project: string;
  workspace: string;
  version: string;
  close: () => void;
  complete: () => void;
}) {
  const [pending, setPending] = useState(() =>
    readPublication(sessionStorage.getItem("gl.publication")),
  );
  const [message, setMessage] = useState(pending?.message ?? "");
  const [result, setResult] = useState<{
    revision: string;
    changes?: string;
  }>();
  const [info, setInfo] = useState<Preflight>();
  const [checking, setChecking] = useState(true);
  const [recheck, setRecheck] = useState(0);
  const [resolving, setResolving] = useState(false);
  const [listOpen, setListOpen] = useState(false);
  const task = useAction();
  const matches =
    !pending ||
    (pending.project === project && pending.workspace === workspace);

  // Best-effort context: a retry of a recorded publication never waits on it.
  const preflight = useCallback(async () => {
    setChecking(true);
    const settle = <T,>(promise: Promise<T>) => promise.catch(() => undefined);
    const [ws, proj, summary] = await Promise.all([
      settle(call<Workspace>({ action: "workspace", project, workspace })),
      settle(call<Project>({ action: "project", project })),
      settle(
        call<ChangeSummary>({ action: "workspaceSummary", project, workspace }),
      ),
    ]);
    const conflicts =
      ws?.status === "open"
        ? await settle(
            call<Conflicts>({
              action: "conflicts",
              project,
              workspace,
              limit: 1,
            }),
          )
        : undefined;
    setInfo({
      workspace: ws,
      head: conflicts?.head ?? proj?.head,
      summary,
      conflicts: conflicts ? Number(conflicts.total) : undefined,
    });
    setChecking(false);
  }, [project, workspace]);
  useEffect(() => {
    void preflight();
  }, [preflight, recheck]);

  const total = info?.summary?.total;
  const empty =
    !!total &&
    String(total.added) === "0" &&
    String(total.modified) === "0" &&
    String(total.deleted) === "0";
  const conflicts = info?.conflicts ?? 0;
  const blocked = !pending && (conflicts > 0 || empty);
  const base = info?.workspace?.baseRevision;
  const head = info?.head;
  const target = head !== undefined ? `r${BigInt(head) + 1n}` : undefined;
  const behind =
    base !== undefined && head !== undefined && BigInt(head) > BigInt(base);

  const submit = () =>
    void task.run(async () => {
      const intent =
        pending ?? publication(project, workspace, version, message.trim());
      // Persist before sending: an interrupted response can only retry this immutable request.
      sessionStorage.setItem("gl.publication", JSON.stringify(intent));
      setPending(intent);
      try {
        const response = await call<{ revision: string; changes?: string }>(
          intent,
        );
        sessionStorage.removeItem("gl.publication");
        setPending(undefined);
        setResult(response);
        toast.success(`已发布 r${response.revision}`);
        complete();
      } catch (error) {
        if (
          error instanceof ApiError &&
          releasePublication(error.status, error.uncertain, !!pending)
        ) {
          sessionStorage.removeItem("gl.publication");
          setPending(undefined);
        }
        if (error instanceof ApiError && error.status === 409)
          setRecheck((n) => n + 1);
        throw error;
      }
    });

  if (result)
    return (
      <Modal
        title="发布完成"
        size="sm"
        close={close}
        footer={
          <>
            <Button variant="outline" asChild>
              <Link
                href={`/history?project=${encodeURIComponent(project)}&revision=${result.revision}`}
                onClick={close}
              >
                <ExternalLink />
                查看版本
              </Link>
            </Button>
            <Button onClick={close} autoFocus>
              完成
            </Button>
          </>
        }
      >
        <div className="publish-success">
          <Check />
          <h3>版本 r{result.revision} 已发布</h3>
          <div className="publish-success-meta">
            <Badge variant="outline" className="revision-tag">
              r{result.revision}
            </Badge>
            <CopyButton value={`r${result.revision}`} label="复制版本号" />
            {result.changes !== undefined && (
              <span className="muted">{count(result.changes)} 个要素</span>
            )}
          </div>
        </div>
      </Modal>
    );

  return (
    <>
      <Modal
        title="发布到新版本"
        size="lg"
        busy={task.busy}
        close={close}
        meta={
          <Badge variant="outline" className="meta-chip">
            <span className="mono">{short(workspace)}</span>
          </Badge>
        }
        footer={
          <>
            <FooterStart>
              {checking && !pending ? (
                <span className="footer-note muted">
                  <RefreshCw className="spin" aria-hidden="true" />
                  检查中
                </span>
              ) : conflicts > 0 && !pending ? (
                <span className="footer-note text-warning">
                  <TriangleAlert aria-hidden="true" />
                  {count(conflicts)} 个冲突
                </span>
              ) : null}
            </FooterStart>
            <Button
              type="button"
              variant="outline"
              disabled={task.busy}
              onClick={close}
            >
              关闭
            </Button>
            <Button
              type="submit"
              form="publish-form"
              disabled={task.busy || !matches || !message.trim() || blocked}
            >
              {task.busy ? <RefreshCw className="spin" /> : <Upload />}
              {task.busy ? "正在发布…" : pending ? "重试原发布" : "确认发布"}
            </Button>
          </>
        }
      >
        <form
          id="publish-form"
          className="dialog-stack"
          onSubmit={(e) => {
            e.preventDefault();
            if (!task.busy && matches && message.trim() && !blocked) submit();
          }}
        >
          <div className="rev-summary">
            <div className="rev-flow">
              <span className="rev-node">
                <span className="rev-label">工作区</span>
                <Badge variant="outline" className="revision-tag">
                  v{info?.workspace?.version ?? version}
                </Badge>
                {base !== undefined && (
                  <span className="muted">基于 r{base}</span>
                )}
              </span>
              <ArrowRight aria-hidden="true" />
              <span className="rev-node">
                <span className="rev-label">发布为</span>
                {target ? (
                  <Badge variant="outline" className="revision-tag is-head">
                    {target}
                  </Badge>
                ) : (
                  <Skeleton className="h-5 w-10" />
                )}
              </span>
              {behind && !pending && conflicts === 0 && (
                <Badge variant="secondary" className="meta-chip">
                  <GitMerge aria-hidden="true" />
                  合并 r{(BigInt(base!) + 1n).toString()}
                  {BigInt(head!) > BigInt(base!) + 1n ? `–r${head}` : ""}
                </Badge>
              )}
            </div>
            {total ? (
              <DiffStat
                added={total.added}
                modified={total.modified}
                deleted={total.deleted}
              />
            ) : checking ? (
              <Skeleton className="h-5 w-28" />
            ) : null}
          </div>
          {conflicts > 0 && !pending && (
            <Notice
              tone="warning"
              icon={<TriangleAlert aria-hidden="true" />}
              action={
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  onClick={() => setResolving(true)}
                >
                  <GitMerge />
                  解决冲突
                </Button>
              }
            >
              {count(conflicts)} 个要素与 r{head} 冲突
            </Notice>
          )}
          {empty && !pending && <Notice tone="muted">无要素变更</Notice>}
          {info?.summary && info.summary.datasets.length > 0 && (
            <ul className="dataset-stats" aria-label="按数据集统计要素变更">
              {info.summary.datasets.map((row) => (
                <li key={row.id}>
                  <span title={row.id}>{row.name}</span>
                  <DiffStat
                    added={row.added}
                    modified={row.modified}
                    deleted={row.deleted}
                  />
                </li>
              ))}
            </ul>
          )}
          {!empty && (
            <Collapsible open={listOpen} onOpenChange={setListOpen}>
              <CollapsibleTrigger asChild>
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  className="disclosure"
                >
                  <ChevronRight className="change-chevron" />
                  变更明细
                </Button>
              </CollapsibleTrigger>
              <CollapsibleContent>
                {listOpen && (
                  <ChangeList
                    project={project}
                    beforeLabel="基线"
                    afterLabel="工作区"
                    reloadKey={recheck}
                    load={async (after) => {
                      const r = await call<Changes>({
                        action: "diff",
                        project,
                        workspace,
                        after,
                        limit: 50,
                      });
                      return {
                        rows: r.changes,
                        next:
                          r.changes.length === 50
                            ? r.changes.at(-1)!.cursor
                            : undefined,
                      };
                    }}
                  />
                )}
              </CollapsibleContent>
            </Collapsible>
          )}
          <div className="field">
            <Label htmlFor="publish-message">版本说明</Label>
            <Textarea
              id="publish-message"
              required
              maxLength={2048}
              rows={3}
              value={message}
              readOnly={!!pending}
              autoFocus={!pending}
              onChange={(e) => setMessage(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
                  e.preventDefault();
                  e.currentTarget.form?.requestSubmit();
                }
              }}
              placeholder="例如：更新道路边界与分类"
            />
          </div>
          {pending && (
            <Notice tone="warning" icon={<TriangleAlert aria-hidden="true" />}>
              {matches
                ? "待确认的发布请求，重试将沿用原内容与请求标识"
                : `请先在工作区 ${short(pending.workspace)} 确认上一笔发布`}
            </Notice>
          )}
          <ErrorBox message={task.error} />
        </form>
      </Modal>
      {resolving && info?.workspace && (
        <Resolution
          project={project}
          workspace={info.workspace}
          mode="resolve"
          close={() => {
            setResolving(false);
            setRecheck((n) => n + 1);
          }}
          complete={() => {
            setResolving(false);
            setRecheck((n) => n + 1);
          }}
        />
      )}
    </>
  );
}

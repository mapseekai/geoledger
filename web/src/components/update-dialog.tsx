"use client";
import {
  ApiError,
  call,
  type Changes,
  type Commit,
  type Conflicts,
  type Workspace,
} from "@/lib/browser-api";
import type { ChangeSummary } from "@/lib/change-summary";
import { count } from "@/lib/format";
import { readPublication } from "@/lib/publication";
import {
  ArrowRight,
  ChevronRight,
  CircleCheck,
  GitMerge,
  RefreshCw,
  TriangleAlert,
} from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { toast } from "sonner";
import { ChangeList } from "./change-list";
import {
  DiffStat,
  Empty,
  ErrorBox,
  FooterStart,
  ListSkeleton,
  Modal,
  Notice,
  RelativeTime,
} from "./common";
import { short } from "./resource-shared";
import { Badge } from "./ui/badge";
import { Button } from "./ui/button";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "./ui/collapsible";

type Upstream = {
  head: string;
  version: string;
  commits: (Commit & { summary?: ChangeSummary })[];
  more: boolean;
  conflicts: number;
};
const MAX_COMMITS = 50;

/** Shows what was published since the workspace base and whether it applies cleanly. */
export function UpdateDialog({
  project,
  workspace,
  close,
  complete,
  resolve,
}: {
  project: string;
  workspace: Workspace;
  close: () => void;
  complete: (revision: string) => void;
  resolve: () => void;
}) {
  const [data, setData] = useState<Upstream>();
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [submitError, setSubmitError] = useState("");
  const [busy, setBusy] = useState(false);
  const [reload, setReload] = useState(0);
  const load = useCallback(async () => {
    setLoading(true);
    setError("");
    try {
      const conflicts = await call<Conflicts>({
        action: "conflicts",
        project,
        workspace: workspace.id,
        limit: 1,
      });
      const commits = await call<Commit[]>({
        action: "history",
        project,
        after: workspace.baseRevision,
        limit: MAX_COMMITS,
      });
      const summaries = await Promise.all(
        commits.map((commit) =>
          call<ChangeSummary>({
            action: "commitSummary",
            project,
            revision: commit.revision,
          }).catch(() => undefined),
        ),
      );
      setData({
        head: conflicts.head,
        version: conflicts.version,
        commits: commits.map((commit, index) => ({
          ...commit,
          summary: summaries[index],
        })),
        more: commits.length === MAX_COMMITS,
        conflicts: Number(conflicts.total),
      });
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "加载失败");
    } finally {
      setLoading(false);
    }
  }, [project, workspace.id, workspace.baseRevision]);
  useEffect(() => {
    void load();
  }, [load, reload]);

  const upToDate = !!data && data.head === workspace.baseRevision;
  const totals = (data?.commits ?? []).reduce(
    (sum, commit) => {
      const t = commit.summary?.total;
      if (!t) return sum;
      return {
        added: sum.added + Number(t.added),
        modified: sum.modified + Number(t.modified),
        deleted: sum.deleted + Number(t.deleted),
      };
    },
    { added: 0, modified: 0, deleted: 0 },
  );
  const update = async () => {
    if (!data) return;
    if (readPublication(sessionStorage.getItem("gl.publication"))) {
      setSubmitError("请先确认待处理的发布请求。");
      return;
    }
    setBusy(true);
    setSubmitError("");
    try {
      const result = await call<{ baseRevision: string }>({
        action: "rebase",
        project,
        workspace: workspace.id,
        version: data.version,
        head: data.head,
        edits: [],
      });
      toast.success(`已更新到 r${result.baseRevision}`);
      complete(result.baseRevision);
    } catch (cause) {
      setSubmitError(cause instanceof Error ? cause.message : "更新失败");
      if (cause instanceof ApiError && cause.status === 409)
        setReload((n) => n + 1);
    } finally {
      setBusy(false);
    }
  };
  return (
    <Modal
      title="更新工作区"
      size="lg"
      busy={busy}
      close={close}
      meta={
        <Badge variant="outline" className="meta-chip">
          <span className="mono">{short(workspace.id)}</span>
        </Badge>
      }
      footer={
        <>
          <FooterStart>
            {data && !upToDate && (
              <span className="footer-note">
                {data.conflicts ? (
                  <span className="text-warning">
                    <TriangleAlert aria-hidden="true" />
                    {count(data.conflicts)} 个冲突
                  </span>
                ) : (
                  <span className="text-success">
                    <CircleCheck aria-hidden="true" />
                    可直接更新
                  </span>
                )}
              </span>
            )}
          </FooterStart>
          <Button variant="outline" disabled={busy} onClick={close}>
            {upToDate ? "关闭" : "取消"}
          </Button>
          {data &&
            !upToDate &&
            (data.conflicts ? (
              <Button onClick={resolve} disabled={busy}>
                <GitMerge />
                解决冲突
              </Button>
            ) : (
              <Button onClick={() => void update()} disabled={busy}>
                {busy ? <RefreshCw className="spin" /> : <RefreshCw />}
                更新到 r{data.head}
              </Button>
            ))}
        </>
      }
    >
      <ErrorBox message={error} retry={() => setReload((n) => n + 1)} />
      <ErrorBox message={submitError} />
      {loading && !data ? (
        <ListSkeleton rows={3} />
      ) : data && upToDate ? (
        <Empty icon={CircleCheck} title={`已是最新 · r${data.head}`} />
      ) : data ? (
        <div className="dialog-stack">
          <div className="rev-summary">
            <div className="rev-flow">
              <Badge variant="outline" className="revision-tag">
                r{workspace.baseRevision}
              </Badge>
              <ArrowRight aria-hidden="true" />
              <Badge variant="outline" className="revision-tag is-head">
                r{data.head}
              </Badge>
              <span className="muted">
                {count(data.commits.length)}
                {data.more ? "+" : ""} 个版本
              </span>
            </div>
            <DiffStat {...totals} />
          </div>
          {data.conflicts > 0 && (
            <Notice
              tone="warning"
              icon={<TriangleAlert aria-hidden="true" />}
              action={
                <Button variant="outline" size="sm" onClick={resolve}>
                  查看冲突
                </Button>
              }
            >
              {count(data.conflicts)} 个要素与已发布版本冲突
            </Notice>
          )}
          <ul className="commit-list" aria-label="上游版本">
            {data.commits.map((commit) => (
              <UpstreamCommit
                key={commit.revision}
                project={project}
                commit={commit}
              />
            ))}
          </ul>
        </div>
      ) : null}
    </Modal>
  );
}

function UpstreamCommit({
  project,
  commit,
}: {
  project: string;
  commit: Commit & { summary?: ChangeSummary };
}) {
  const [open, setOpen] = useState(false);
  const t = commit.summary?.total;
  return (
    <li>
      <Collapsible open={open} onOpenChange={setOpen}>
        <CollapsibleTrigger className="commit-row">
          <ChevronRight className="change-chevron" aria-hidden="true" />
          <Badge variant="outline" className="revision-tag">
            r{commit.revision}
          </Badge>
          <span className="commit-message">{commit.message}</span>
          <span className="commit-meta">
            <span>{commit.subject}</span>
            <RelativeTime value={commit.createdAt} plain />
          </span>
          {t ? (
            <DiffStat
              added={t.added}
              modified={t.modified}
              deleted={t.deleted}
            />
          ) : (
            <span />
          )}
        </CollapsibleTrigger>
        <CollapsibleContent>
          {open && (
            <ChangeList
              className="is-nested"
              project={project}
              load={async (after) => {
                const result = await call<Changes>({
                  action: "commit",
                  project,
                  revision: commit.revision,
                  after,
                  limit: 50,
                });
                return {
                  rows: result.changes,
                  next:
                    result.changes.length === 50
                      ? result.changes.at(-1)!.cursor
                      : undefined,
                };
              }}
            />
          )}
        </CollapsibleContent>
      </Collapsible>
    </li>
  );
}

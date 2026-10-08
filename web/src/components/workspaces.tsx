"use client";
import {
  call,
  type Changes,
  type Conflicts,
  type Project,
  type Workspace,
} from "@/lib/browser-api";
import { pretty } from "@/lib/geojson";
import { readPublication, type Publication } from "@/lib/publication";
import {
  Ellipsis,
  FileDiff,
  GitBranch,
  GitMerge,
  Plus,
  RefreshCcw,
  Search,
  Trash2,
  TriangleAlert,
  Upload,
} from "lucide-react";
import { useEffect, useState } from "react";
import { Empty, ErrorBox, Loading, Modal, listPage, usePage } from "./common";
import { PublishDialog } from "./features";
import {
  Panel,
  PanelTitle,
  StatusBadge,
  short,
  useAction,
} from "./resource-shared";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "./ui/dropdown-menu";
import { Button } from "./ui/button";
import { Input } from "./ui/input";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "./ui/table";
import { Textarea } from "./ui/textarea";
export function Inspector({
  title,
  description,
  load,
  close,
}: {
  title: string;
  description: string;
  load: (
    after: string,
  ) => Promise<{ rows: { cursor: string; json: string }[]; next?: string }>;
  close: () => void;
}) {
  const page = usePage(load, 0);
  return (
    <Modal title={title} description={description} close={close}>
      <ErrorBox message={page.error} />
      {page.busy ? (
        <Loading />
      ) : page.rows.length ? (
        <div className="inspect-list">
          {page.rows.map((r) => (
            <pre key={r.cursor} className="json-view">
              {pretty(r.json)}
            </pre>
          ))}
        </div>
      ) : (
        <Empty icon={FileDiff} title="没有变更记录">
          当前页面没有内容。
        </Empty>
      )}
      {page.footer}
    </Modal>
  );
}
export function Workspaces({
  project,
  writable,
}: {
  project: Project;
  writable: boolean;
}) {
  const [refresh, setRefresh] = useState(0),
    [inspect, setInspect] = useState<{
      workspace: Workspace;
      mode: "diff" | "conflicts";
    }>(),
    [publish, setPublish] = useState<Workspace>(),
    [discard, setDiscard] = useState<Workspace>(),
    [resolve, setResolve] = useState<{
      workspace: Workspace;
      mode: "resolve" | "rebase";
    }>();
  const [lookup, setLookup] = useState<Workspace>(),
    [lookupId, setLookupId] = useState("");
  const [pending, setPending] = useState<Publication>();
  useEffect(() => {
    setPending(readPublication(sessionStorage.getItem("gl.publication")));
  }, []);
  const task = useAction();
  const page = usePage<Workspace>(
    async (after) =>
      listPage(
        await call<Workspace[]>({
          action: "workspaces",
          project: project.id,
          after,
          limit: 20,
        }),
      ),
    refresh,
  );
  const rows = lookup ? [lookup] : page.rows;
  const update = () => {
    setLookup(undefined);
    setRefresh((n) => n + 1);
    page.reset();
  };
  return (
    <>
      {pending && (
        <div className="notice is-warning">
          <TriangleAlert aria-hidden="true" />
          <span className="notice-text">
            有一笔待确认的发布：
            <span className="mono">{short(pending.workspace)}</span>。
          </span>
          <Button
            variant="outline"
            size="sm"
            onClick={() =>
              setPublish({
                id: pending.workspace,
                version: pending.version,
                baseRevision: "0",
                status: "open",
              })
            }
          >
            重试原发布
          </Button>
        </div>
      )}
      <Panel
        toolbar={
          <>
            <PanelTitle title="工作区列表" />
            <div className="toolbar-actions">
              <form
                className="lookup"
                onSubmit={(e) => {
                  e.preventDefault();
                  void task.run(async () =>
                    setLookup(
                      await call<Workspace>({
                        action: "workspace",
                        project: project.id,
                        workspace: lookupId.trim(),
                      }),
                    ),
                  );
                }}
              >
                <div className="search">
                  <Search size={15} aria-hidden="true" />
                  <Input
                    aria-label="工作区标识"
                    value={lookupId}
                    onChange={(e) => setLookupId(e.target.value)}
                    placeholder="按工作区标识查找"
                    required
                  />
                </div>
                <Button variant="outline" disabled={task.busy}>
                  查找
                </Button>
                {lookup && (
                  <Button
                    type="button"
                    variant="ghost"
                    onClick={() => setLookup(undefined)}
                  >
                    重置
                  </Button>
                )}
              </form>
              <Button
                disabled={!writable || task.busy}
                onClick={() =>
                  void task.run(async () => {
                    await call({
                      action: "createWorkspace",
                      project: project.id,
                    });
                    update();
                  })
                }
              >
                <Plus />
                新建工作区
              </Button>
            </div>
          </>
        }
      >
        <ErrorBox message={task.error || page.error} />
        {page.busy ? (
          <Loading />
        ) : rows.length ? (
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>工作区</TableHead>
                <TableHead>基准 / 编辑版本</TableHead>
                <TableHead>状态</TableHead>
                <TableHead className="cell-actions">操作</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {rows.map((w) => (
                <TableRow key={w.id}>
                  <TableCell>
                    <span className="id-cell">
                      <span className="row-icon is-branch" aria-hidden="true">
                        <GitBranch size={15} />
                      </span>
                      <span className="mono" title={w.id}>
                        {short(w.id)}
                      </span>
                      <button
                        className="copy-id"
                        onClick={() => setLookupId(w.id)}
                        aria-label={`选择工作区 ${w.id}`}
                      >
                        选择
                      </button>
                    </span>
                  </TableCell>
                  <TableCell className="mono">
                    <span className="revision-tag">r{w.baseRevision}</span>
                    <span className="muted"> / </span>v{w.version}
                  </TableCell>
                  <TableCell>
                    <StatusBadge value={w.status} />
                  </TableCell>
                  <TableCell className="cell-actions">
                    <div className="row-actions">
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() =>
                          setInspect({ workspace: w, mode: "diff" })
                        }
                      >
                        变更
                      </Button>
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() =>
                          setInspect({ workspace: w, mode: "conflicts" })
                        }
                      >
                        冲突
                      </Button>
                      {w.status === "open" && writable && (
                        <>
                          <Button
                            variant="outline"
                            size="sm"
                            onClick={() => setPublish(w)}
                          >
                            <Upload />
                            发布
                          </Button>
                          <DropdownMenu>
                            <DropdownMenuTrigger asChild>
                              <Button
                                variant="ghost"
                                size="icon-sm"
                                aria-label={`更多操作 ${w.id}`}
                                title="更多操作"
                              >
                                <Ellipsis />
                              </Button>
                            </DropdownMenuTrigger>
                            <DropdownMenuContent align="end" className="menu">
                              <DropdownMenuItem
                                onSelect={() =>
                                  setResolve({ workspace: w, mode: "resolve" })
                                }
                              >
                                <GitMerge />
                                解决冲突
                              </DropdownMenuItem>
                              <DropdownMenuItem
                                onSelect={() =>
                                  setResolve({ workspace: w, mode: "rebase" })
                                }
                              >
                                <RefreshCcw />
                                更新基准
                              </DropdownMenuItem>
                              <DropdownMenuSeparator />
                              <DropdownMenuItem
                                variant="destructive"
                                onSelect={() => setDiscard(w)}
                              >
                                <Trash2 />
                                丢弃工作区
                              </DropdownMenuItem>
                            </DropdownMenuContent>
                          </DropdownMenu>
                        </>
                      )}
                    </div>
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        ) : (
          <Empty icon={GitBranch} title="还没有工作区" />
        )}
        {!lookup && page.footer}
      </Panel>
      {inspect && (
        <Inspector
          key={`${inspect.workspace.id}:${inspect.mode}`}
          title={inspect.mode === "diff" ? "工作区变更" : "工作区冲突"}
          description={`工作区 ${inspect.workspace.id}`}
          close={() => setInspect(undefined)}
          load={async (after) => {
            if (inspect.mode === "diff") {
              const r = await call<Changes>({
                action: "diff",
                project: project.id,
                workspace: inspect.workspace.id,
                after,
                limit: 20,
              });
              return {
                rows: r.changes,
                identity: r.version,
                next:
                  r.changes.length === 20
                    ? r.changes.at(-1)!.cursor
                    : undefined,
              };
            }
            const r = await call<Conflicts>({
              action: "conflicts",
              project: project.id,
              workspace: inspect.workspace.id,
              after,
              limit: 20,
            });
            return {
              rows: r.conflicts,
              next: r.nextAfter,
              identity: `${r.version}:${r.head}`,
            };
          }}
        />
      )}
      {publish && (
        <PublishDialog
          project={
            pending?.workspace === publish.id ? pending.project : project.id
          }
          workspace={publish.id}
          version={publish.version}
          close={() => {
            setPublish(undefined);
            setPending(
              readPublication(sessionStorage.getItem("gl.publication")),
            );
          }}
          complete={() => {
            setPending(undefined);
            update();
          }}
        />
      )}
      {discard && (
        <Modal
          title="丢弃工作区"
          description="工作区的未发布编辑将被丢弃。已发布的历史版本不会改变。"
          close={task.busy ? () => {} : () => setDiscard(undefined)}
        >
          <ErrorBox message={task.error} />
          <div className="form-actions">
            <Button
              variant="outline"
              onClick={() => setDiscard(undefined)}
              disabled={task.busy}
            >
              取消
            </Button>
            <Button
              variant="destructive"
              disabled={task.busy}
              onClick={() =>
                void task.run(async () => {
                  if (readPublication(sessionStorage.getItem("gl.publication")))
                    throw new Error("请先确认待处理的发布请求。");
                  await call({
                    action: "discard",
                    project: project.id,
                    workspace: discard.id,
                    version: discard.version,
                  });
                  setDiscard(undefined);
                  update();
                })
              }
            >
              确认丢弃
            </Button>
          </div>
        </Modal>
      )}
      {resolve && (
        <Resolution
          project={project.id}
          workspace={resolve.workspace}
          mode={resolve.mode}
          close={() => setResolve(undefined)}
          complete={() => {
            setResolve(undefined);
            update();
          }}
        />
      )}
    </>
  );
}
export function Resolution({
  project,
  workspace,
  mode,
  close,
  complete,
}: {
  project: string;
  workspace: Workspace;
  mode: "resolve" | "rebase";
  close: () => void;
  complete: () => void;
}) {
  const task = useAction();
  const [head, setHead] = useState<string>(),
    [error, setError] = useState("");
  useEffect(() => {
    let active = true;
    call<Project>({ action: "project", project })
      .then((p) => {
        if (active) setHead(p.head);
      })
      .catch((e) => {
        if (active) setError(e.message);
      });
    return () => {
      active = false;
    };
  }, [project]);
  return (
    <Modal
      title={mode === "resolve" ? "解决冲突" : "更新工作区基准"}
      description={`以项目版本 r${head ?? "…"} 为目标。版本变化时会拒绝本次操作，请重新打开窗口。`}
      close={task.busy ? () => {} : close}
    >
      <form
        onSubmit={(e) => {
          e.preventDefault();
          const raw = String(new FormData(e.currentTarget).get("resolutions"));
          void task.run(async () => {
            if (readPublication(sessionStorage.getItem("gl.publication")))
              throw new Error("请先确认待处理的发布请求。");
            const edits = JSON.parse(raw);
            await call({
              action: mode,
              project,
              workspace: workspace.id,
              version: workspace.version,
              head,
              edits,
            });
            complete();
          });
        }}
      >
        <p className="field-help">
          填写编辑数组；每项包含 dataset、featureId、feature。feature 使用
          GeoJSON 文本字符串，或使用 null
          删除。更新基准时可使用空数组自动合并无冲突变化。
        </p>
        <label htmlFor="resolutions">解决方案</label>
        <Textarea
          id="resolutions"
          name="resolutions"
          rows={10}
          defaultValue="[]"
          className="code-editor"
          required
        />
        <ErrorBox message={error || task.error} />
        <div className="form-actions">
          <Button
            variant="outline"
            type="button"
            disabled={task.busy}
            onClick={close}
          >
            取消
          </Button>
          <Button disabled={task.busy || head === undefined}>
            确认{mode === "resolve" ? "解决" : "更新"}
          </Button>
        </div>
      </form>
    </Modal>
  );
}

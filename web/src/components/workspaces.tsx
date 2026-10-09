"use client";
import {
  call,
  type Changes,
  type Commit,
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
import { useEffect, useState, type ReactNode } from "react";
import {
  Confirm,
  Empty,
  ErrorBox,
  Loading,
  Modal,
  Notice,
  Tip,
  listPage,
  usePage,
} from "./common";
import { ChangeSummary } from "./change-summary";
import { Resolution } from "./conflict-resolution";
import { PublishDialog } from "./features";
import { VersionGraph } from "./version-graph";
import { Tabs, TabsList, TabsTrigger, TabsContent } from "./ui/tabs";
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
import { Badge } from "./ui/badge";
import { Button } from "./ui/button";
import { Input } from "./ui/input";
import {
  Table,
  TableCaption,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "./ui/table";

export function Inspector({
  title,
  description,
  summary,
  load,
  close,
}: {
  title: string;
  description: string;
  summary?: ReactNode;
  load: (
    after: string,
  ) => Promise<{ rows: { cursor: string; json: string }[]; next?: string }>;
  close: () => void;
}) {
  const page = usePage(load, 0);
  return (
    <Modal title={title} description={description} close={close}>
      {summary}
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
  const [refresh, setRefresh] = useState(0);
  const [view, setView] = useState<"graph" | "table">("graph");
  const [inspect, setInspect] = useState<
    { workspace: Workspace } | { revision: string; commit?: Commit }
  >();
  const [publish, setPublish] = useState<Workspace>();
  const [discard, setDiscard] = useState<Workspace>();
  const [resolve, setResolve] = useState<{
    workspace: Workspace;
    mode: "resolve" | "rebase";
  }>();
  const [lookup, setLookup] = useState<Workspace>();
  const [lookupId, setLookupId] = useState("");
  const [selectedProject, setSelectedProject] = useState(project.id);
  const [selected, setSelected] = useState<Workspace>();
  const [pending, setPending] = useState<Publication>();
  const [refreshError, setRefreshError] = useState("");
  const [reconciling, setReconciling] = useState(false);
  const [graphProject, setGraphProject] = useState(project);
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
    `${project.id}:${refresh}`,
  );

  useEffect(() => {
    setPending(readPublication(sessionStorage.getItem("gl.publication")));
  }, []);
  useEffect(() => {
    let active = true;
    call<Project>({ action: "project", project: project.id })
      .then((next) => {
        if (active) {
          setGraphProject(next);
          setRefreshError("");
        }
      })
      .catch((error) => {
        if (active)
          setRefreshError(
            error instanceof Error
              ? `刷新 HEAD 失败：${error.message}`
              : "刷新 HEAD 失败。",
          );
      });
    return () => {
      active = false;
    };
  }, [project.id, refresh]);
  useEffect(() => {
    setLookup(undefined);
    setSelected(undefined);
    setSelectedProject(project.id);
    setView("graph");
  }, [project.id]);
  useEffect(() => {
    if (!reconciling && !selected && page.rows[0])
      setSelected(
        page.rows.find((workspace) => workspace.status === "open") ??
          page.rows[0],
      );
  }, [page.rows, reconciling, selected]);

  const rows = lookup ? [lookup] : page.rows;
  const graphWorkspaces = lookup
    ? [lookup, ...page.rows.filter((workspace) => workspace.id !== lookup.id)]
    : page.rows;
  const currentSelected = selectedProject === project.id ? selected : undefined;
  const currentGraphProject =
    graphProject.id === project.id ? graphProject : project;
  const update = async () => {
    const selectedId = selected?.id;
    const lookupWorkspaceId = lookup?.id;
    if (selectedId || lookupWorkspaceId) setReconciling(true);
    setSelected(undefined);
    setLookup(undefined);
    setRefreshError("");
    setRefresh((n) => n + 1);
    page.reset();
    if (!selectedId && !lookupWorkspaceId) return;
    try {
      const refreshed = await Promise.all(
        [
          ...new Set(
            [selectedId, lookupWorkspaceId].filter((id): id is string =>
              Boolean(id),
            ),
          ),
        ].map((workspace) =>
          call<Workspace>({
            action: "workspace",
            project: project.id,
            workspace,
          }),
        ),
      );
      const byId = new Map(
        refreshed.map((workspace) => [workspace.id, workspace]),
      );
      if (selectedId) setSelected(byId.get(selectedId));
      if (lookupWorkspaceId) setLookup(byId.get(lookupWorkspaceId));
    } catch (error) {
      setRefreshError(
        error instanceof Error
          ? `刷新工作区失败：${error.message}`
          : "刷新工作区失败。",
      );
    } finally {
      setReconciling(false);
    }
  };
  const selectWorkspace = (workspace: Workspace) => {
    setSelected(workspace);
  };
  const workspaceActions = (workspace: Workspace): ReactNode => (
    <div className="row-actions">
      <Button
        variant="ghost"
        size="sm"
        onClick={() => setInspect({ workspace })}
      >
        变更
      </Button>
      <Button
        variant="ghost"
        size="sm"
        onClick={() => setResolve({ workspace, mode: "resolve" })}
      >
        冲突
      </Button>
      {workspace.status === "open" && writable && (
        <>
          <Button
            variant="outline"
            size="sm"
            onClick={() => setPublish(workspace)}
          >
            <Upload />
            发布
          </Button>
          <DropdownMenu>
            <Tip label="更多操作">
              <DropdownMenuTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon-sm"
                  aria-label={`更多操作 ${workspace.id}`}
                >
                  <Ellipsis />
                </Button>
              </DropdownMenuTrigger>
            </Tip>
            <DropdownMenuContent align="end" className="menu">
              <DropdownMenuItem
                onSelect={() => setResolve({ workspace, mode: "resolve" })}
              >
                <GitMerge />
                解决冲突
              </DropdownMenuItem>
              <DropdownMenuItem
                onSelect={() => setResolve({ workspace, mode: "rebase" })}
              >
                <RefreshCcw />
                更新基准
              </DropdownMenuItem>
              <DropdownMenuSeparator />
              <DropdownMenuItem
                variant="destructive"
                onSelect={() => setDiscard(workspace)}
              >
                <Trash2 />
                丢弃工作区
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        </>
      )}
    </div>
  );

  return (
    <>
      {pending && (
        <Notice
          tone="warning"
          icon={<TriangleAlert aria-hidden="true" />}
          action={
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
          }
        >
          有一笔待确认的发布：
          <span className="mono">{short(pending.workspace)}</span>。
        </Notice>
      )}
      <Tabs
        value={view}
        onValueChange={(value) => setView(value as "graph" | "table")}
      >
        <Panel
          toolbar={
            <>
              <PanelTitle title="工作区" />
              <div className="toolbar-actions">
                <TabsList aria-label="工作区视图">
                  <TabsTrigger value="graph">版本图</TabsTrigger>
                  <TabsTrigger value="table">表格</TabsTrigger>
                </TabsList>
                <form
                  className="lookup"
                  onSubmit={(event) => {
                    event.preventDefault();
                    void task.run(async () => {
                      const workspace = await call<Workspace>({
                        action: "workspace",
                        project: project.id,
                        workspace: lookupId.trim(),
                      });
                      setLookup(workspace);
                      setSelected(workspace);
                    });
                  }}
                >
                  <div className="search">
                    <Search size={15} aria-hidden="true" />
                    <Input
                      aria-label="工作区标识"
                      value={lookupId}
                      onChange={(event) => setLookupId(event.target.value)}
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
                      const workspace = await call<Workspace>({
                        action: "createWorkspace",
                        project: project.id,
                      });
                      setLookup(workspace);
                      setSelected(workspace);
                      setRefresh((value) => value + 1);
                      page.reset();
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
          <ErrorBox message={task.error || page.error || refreshError} />
          <TabsContent value="graph">
            <VersionGraph
              project={currentGraphProject}
              refresh={refresh}
              workspaces={graphWorkspaces}
              selected={currentSelected}
              chooseWorkspace={selectWorkspace}
              onRefresh={() => void update()}
              onRevision={(revision, commit) =>
                setInspect({ revision, commit })
              }
              actions={workspaceActions}
            />
          </TabsContent>
          <TabsContent value="table">
            {page.busy ? (
              <Loading />
            ) : rows.length ? (
              <Table aria-label="工作区列表">
                <TableCaption className="caption-top p-3 text-left font-medium">
                  工作区列表与数据集变更
                </TableCaption>
                <TableHeader>
                  <TableRow>
                    <TableHead>工作区</TableHead>
                    <TableHead>基准 / 编辑版本</TableHead>
                    <TableHead>状态</TableHead>
                    <TableHead>数据集 / 要素变更</TableHead>
                    <TableHead className="cell-actions">操作</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {rows.map((workspace) => (
                    <TableRow key={workspace.id}>
                      <TableCell>
                        <span className="id-cell">
                          <span
                            className="row-icon is-branch"
                            aria-hidden="true"
                          >
                            <GitBranch size={15} />
                          </span>
                          <span className="mono" title={workspace.id}>
                            {short(workspace.id)}
                          </span>
                          <Button
                            variant="ghost"
                            size="xs"
                            className="copy-id"
                            onClick={() => {
                              setSelected(workspace);
                              setView("graph");
                            }}
                            aria-label={`选择工作区 ${workspace.id}`}
                          >
                            选择
                          </Button>
                        </span>
                      </TableCell>
                      <TableCell className="mono">
                        <Badge variant="outline" className="revision-tag">
                          r{workspace.baseRevision}
                        </Badge>
                        <span className="muted"> / </span>v{workspace.version}
                      </TableCell>
                      <TableCell>
                        <StatusBadge value={workspace.status} />
                      </TableCell>
                      <TableCell>
                        <ChangeSummary
                          project={project.id}
                          workspace={workspace.id}
                          refresh={`${workspace.version}:${refresh}`}
                          compact
                        />
                      </TableCell>
                      <TableCell className="cell-actions">
                        {workspaceActions(workspace)}
                      </TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            ) : (
              <Empty icon={GitBranch} title="还没有工作区" />
            )}
          </TabsContent>
          {!lookup && page.footer}
        </Panel>
      </Tabs>
      {inspect && (
        <Inspector
          key={"workspace" in inspect ? inspect.workspace.id : inspect.revision}
          title={
            "workspace" in inspect ? "工作区变更" : `版本 r${inspect.revision}`
          }
          description={
            "workspace" in inspect
              ? `工作区 ${inspect.workspace.id}`
              : (inspect.commit?.message ?? "版本变更")
          }
          summary={
            <ChangeSummary
              project={project.id}
              {...("workspace" in inspect
                ? {
                    workspace: inspect.workspace.id,
                    refresh: inspect.workspace.version,
                  }
                : { revision: inspect.revision })}
            />
          }
          close={() => setInspect(undefined)}
          load={async (after) => {
            if ("workspace" in inspect) {
              const result = await call<Changes>({
                action: "diff",
                project: project.id,
                workspace: inspect.workspace.id,
                after,
                limit: 20,
              });
              return {
                rows: result.changes,
                identity: result.version,
                next:
                  result.changes.length === 20
                    ? result.changes.at(-1)!.cursor
                    : undefined,
              };
            }
            const result = await call<Changes>({
              action: "commit",
              project: project.id,
              revision: inspect.revision,
              after,
              limit: 20,
            });
            return {
              rows: result.changes,
              next:
                result.changes.length === 20
                  ? result.changes.at(-1)!.cursor
                  : undefined,
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
            void update();
          }}
        />
      )}
      {discard && (
        <Confirm
          title="丢弃工作区"
          description="工作区的未发布编辑将被丢弃。已发布的历史版本不会改变。"
          action="确认丢弃"
          busy={task.busy}
          error={task.error}
          close={() => setDiscard(undefined)}
          confirm={() =>
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
              await update();
            })
          }
        />
      )}
      {resolve && (
        <Resolution
          project={project.id}
          workspace={resolve.workspace}
          mode={resolve.mode}
          readOnly={!writable || resolve.workspace.status !== "open"}
          close={() => {
            setResolve(undefined);
            void update();
          }}
          complete={() => {
            setResolve(undefined);
            void update();
          }}
        />
      )}
    </>
  );
}

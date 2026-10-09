"use client";
import {
  call,
  type Commit,
  type Project,
  type Workspace,
} from "@/lib/browser-api";
import { ChangeSummary } from "./change-summary";
import { layoutGraph } from "@/lib/version-graph-layout";
import { GitPullRequest } from "lucide-react";
import {
  useEffect,
  useMemo,
  useState,
  type CSSProperties,
  type ReactNode,
} from "react";
import { ErrorBox, Loading } from "./common";
import { Button } from "./ui/button";
import { Popover, PopoverTrigger, PopoverContent } from "./ui/popover";
import styles from "./version-graph.module.css";
import { short, time } from "./resource-shared";

const PAGE_SIZE = 20n;
const ROW_HEIGHT = 64;
const MAINLINE_X = 18;
const LANE_START_X = 48;
const LANE_GAP = 24;
const LANE_COLORS = ["#0891b2", "#eab308", "#7c3aed", "#f97316"];

type HistoryState = { key: string; uppers: string[] };
type GraphRow =
  | { kind: "draft"; workspace: Workspace; base?: string; id: string }
  | { kind: "commit"; commit: Commit; base?: string; id: string };

function oldestFirst(rows: Commit[]) {
  return [...rows].sort((left, right) =>
    BigInt(left.revision) < BigInt(right.revision) ? -1 : 1,
  );
}

function pageMarker(x: number, y: number, boundary: "top" | "bottom") {
  const offset = boundary === "top" ? 5 : -5;
  return `M ${x - 4} ${y + offset} L ${x} ${y} L ${x + 4} ${y + offset}`;
}

export function VersionGraph({
  project,
  refresh,
  workspaces,
  selected,
  chooseWorkspace,
  onRevision,
  onRefresh,
  actions,
}: {
  project: Project;
  refresh: number;
  workspaces: Workspace[];
  selected?: Workspace;
  chooseWorkspace: (workspace: Workspace) => void;
  onRevision: (revision: string, commit?: Commit) => void;
  onRefresh: () => void;
  actions: (workspace: Workspace) => ReactNode;
}) {
  const key = `${project.id}:${project.head}:${refresh}`;
  const [history, setHistory] = useState<HistoryState>({
    key,
    uppers: [project.head],
  });
  const current =
    history.key === key ? history : { key, uppers: [project.head] };
  const upper = current.uppers.at(-1)!;
  const [rows, setRows] = useState<Commit[]>([]);
  const [next, setNext] = useState<string>();
  const [busy, setBusy] = useState(true);
  const [error, setError] = useState("");

  useEffect(() => {
    if (history.key !== key) setHistory({ key, uppers: [project.head] });
  }, [history.key, key, project.head]);

  useEffect(() => {
    let active = true;
    setBusy(true);
    setError("");
    setRows([]);
    setNext(undefined);
    const load = async () => {
      const ceiling = BigInt(upper);
      if (ceiling === 0n) return { rows: [] as Commit[], next: undefined };
      const after = ceiling > PAGE_SIZE ? ceiling - PAGE_SIZE : 0n;
      const fetched = await call<Commit[]>({
        action: "history",
        project: project.id,
        after: after.toString(),
        limit: Number(PAGE_SIZE),
      });
      const page = oldestFirst(
        fetched.filter((commit) => BigInt(commit.revision) <= ceiling),
      );
      const oldest = page.at(0);
      return {
        rows: page,
        next:
          oldest && BigInt(oldest.revision) > 1n
            ? (BigInt(oldest.revision) - 1n).toString()
            : undefined,
      };
    };
    load()
      .then((page) => {
        if (active) {
          setRows(page.rows);
          setNext(page.next);
        }
      })
      .catch((reason) => {
        if (active)
          setError(reason instanceof Error ? reason.message : "加载版本失败");
      })
      .finally(() => {
        if (active) setBusy(false);
      });
    return () => {
      active = false;
    };
  }, [key, project.id, upper]);

  const graphRows = useMemo<GraphRow[]>(
    () => [
      ...rows.map((commit) => ({
        kind: "commit" as const,
        commit,
        base: commit.sourceWorkspace ? commit.sourceBaseRevision : undefined,
        id: `r${commit.revision}`,
      })),
      ...workspaces
        .filter((workspace) => workspace.status === "open")
        .map((workspace) => ({
          kind: "draft" as const,
          workspace,
          base: workspace.baseRevision,
          id: `draft:${workspace.id}`,
        })),
    ],
    [rows, workspaces],
  );
  const layout = useMemo(
    () =>
      layoutGraph(
        graphRows.map((row) => ({
          id: row.id,
          base: row.base,
          draft: row.kind === "draft",
        })),
      ),
    [graphRows],
  );
  const width = LANE_START_X + layout.laneCount * LANE_GAP + 14;
  const nodeByRow = layout.nodes;
  const selectedBaseOnPage =
    selected &&
    rows.some((commit) => commit.revision === selected.baseRevision);

  return (
    <section className={styles.graph} aria-label="版本图">
      {error ? (
        <ErrorBox message={error} />
      ) : busy ? (
        <Loading rows={3} />
      ) : !graphRows.length ? (
        <p className={styles.empty}>还没有发布记录或进行中的工作区。</p>
      ) : (
        <div className={styles.graphScroll}>
          <div
            className={styles.canvas}
            style={
              {
                "--rail-width": `${width}px`,
                "--row-count": graphRows.length,
                "--row-height": `${ROW_HEIGHT}px`,
              } as CSSProperties
            }
          >
            <div
              className={`${styles.row} ${styles.columnHeaders}`}
              aria-label="版本图表头"
            >
              <span>分支</span>
              <span>版本 / 工作区</span>
              <span>说明</span>
              <span>发布者 / 基线</span>
              <span>时间 / 基线位置</span>
              <span>数据集 / 要素变更</span>
              <span>来源 / 操作</span>
            </div>
            <svg
              className={styles.lines}
              aria-hidden="true"
              viewBox={`0 0 ${width} ${graphRows.length * ROW_HEIGHT}`}
              preserveAspectRatio="none"
            >
              {layout.edges.map((edge) => {
                const fromX =
                  edge.fromLane === 0
                    ? MAINLINE_X
                    : LANE_START_X + (edge.fromLane - 1) * LANE_GAP;
                const fromY = edge.fromRow * ROW_HEIGHT + ROW_HEIGHT / 2;
                const toX =
                  edge.toLane === undefined || edge.toLane === 0
                    ? MAINLINE_X
                    : LANE_START_X + (edge.toLane - 1) * LANE_GAP;
                const toY =
                  edge.toRow === undefined
                    ? edge.boundary === "top"
                      ? 1
                      : graphRows.length * ROW_HEIGHT - 1
                    : edge.toRow * ROW_HEIGHT + ROW_HEIGHT / 2;
                const middle = fromY + (toY - fromY) / 2;
                const turn = Math.min(28, Math.abs(toY - fromY) / 2);
                const turnY = toY - Math.sign(toY - fromY) * turn;
                const path =
                  fromX === toX
                    ? `M ${fromX} ${fromY} V ${toY}`
                    : edge.draft
                      ? `M ${fromX} ${fromY} V ${turnY} C ${fromX} ${toY - (Math.sign(toY - fromY) * turn) / 2}, ${toX} ${toY - (Math.sign(toY - fromY) * turn) / 2}, ${toX} ${toY}`
                      : `M ${fromX} ${fromY} C ${fromX} ${middle}, ${toX} ${middle}, ${toX} ${toY}`;
                return (
                  <g
                    className={`${styles.branch} ${edge.draft ? styles.draftBranch : ""}`}
                    key={`${edge.from}:${edge.to ?? edge.boundary}`}
                    style={
                      {
                        "--branch-color":
                          edge.fromLane === 0
                            ? "#a3a3a3"
                            : LANE_COLORS[
                                (edge.fromLane - 1) % LANE_COLORS.length
                              ],
                      } as CSSProperties
                    }
                  >
                    <path d={path} />
                    {edge.boundary && (
                      <>
                        <path
                          className={styles.offPage}
                          d={pageMarker(toX, toY, edge.boundary)}
                        />
                        <text
                          className={styles.offPageLabel}
                          x={toX + 7}
                          y={edge.boundary === "top" ? 12 : toY - 9}
                        >
                          页外基线
                        </text>
                      </>
                    )}
                  </g>
                );
              })}
            </svg>
            <ol className={styles.log}>
              {graphRows.map((row, index) => {
                const node = nodeByRow[index];
                const branchX =
                  node.lane === 0
                    ? MAINLINE_X
                    : LANE_START_X + (node.lane - 1) * LANE_GAP;
                if (row.kind === "draft") {
                  const baseOnPage = rows.some(
                    (commit) => commit.revision === row.workspace.baseRevision,
                  );
                  return (
                    <li
                      className={`${styles.row} ${styles.draft}`}
                      key={row.id}
                    >
                      <span
                        className={styles.node}
                        style={
                          {
                            left: branchX,
                            "--node-color":
                              node.lane === 0
                                ? "var(--brand)"
                                : LANE_COLORS[
                                    (node.lane - 1) % LANE_COLORS.length
                                  ],
                          } as CSSProperties
                        }
                      />
                      <Button
                        variant="ghost"
                        size="sm"
                        className={styles.commit}
                        type="button"
                        onClick={() => chooseWorkspace(row.workspace)}
                      >
                        <strong>草稿</strong>
                        <span>{short(row.workspace.id)}</span>
                      </Button>
                      <span className={styles.message}>未提交</span>
                      <span className={`${styles.meta} ${styles.draftBase}`}>
                        基线 r{row.workspace.baseRevision}
                      </span>
                      <span className={`${styles.meta} ${styles.draftPage}`}>
                        {baseOnPage
                          ? "本页基线"
                          : `r${row.workspace.baseRevision} 页外基线`}
                      </span>
                      <div className={styles.changeCounts}>
                        <ChangeSummary
                          project={project.id}
                          workspace={row.workspace.id}
                          refresh={`${row.workspace.version}:${refresh}`}
                          compact
                        />
                      </div>
                      <span className={styles.rowActions}>
                        {actions(row.workspace)}
                      </span>
                    </li>
                  );
                }
                const { commit } = row;
                const source = workspaces.find(
                  (workspace) => workspace.id === commit.sourceWorkspace,
                );
                const sourceBase = (
                  commit as Commit & { sourceBaseRevision?: string }
                ).sourceBaseRevision;
                const sourceKnown = Boolean(
                  commit.sourceWorkspace && sourceBase !== undefined,
                );
                const sourceBaseOnPage =
                  sourceBase !== undefined &&
                  rows.some((commit) => commit.revision === sourceBase);
                return (
                  <li className={styles.row} key={row.id}>
                    <span
                      className={styles.node}
                      style={{
                        left: branchX,
                        background:
                          node.lane === 0
                            ? "#a3a3a3"
                            : LANE_COLORS[(node.lane - 1) % LANE_COLORS.length],
                      }}
                    />
                    <Button
                      variant="ghost"
                      size="sm"
                      className={styles.commit}
                      type="button"
                      onClick={() => onRevision(commit.revision, commit)}
                      aria-label={`查看版本 r${commit.revision} 的变更`}
                    >
                      <strong>
                        r{commit.revision}
                        {commit.revision === project.head && (
                          <em className={styles.headMark}>HEAD</em>
                        )}
                      </strong>
                    </Button>
                    <span className={styles.message} title={commit.message}>
                      {commit.message || "无说明"}
                    </span>
                    <span className={styles.meta} title={commit.subject}>
                      {commit.subject}
                    </span>
                    <time
                      className={styles.meta}
                      dateTime={commit.createdAt}
                      title={commit.createdAt}
                    >
                      {time(commit.createdAt)}
                    </time>
                    <div className={styles.changeCounts}>
                      <ChangeSummary
                        project={project.id}
                        revision={commit.revision}
                        compact
                      />
                    </div>
                    <div className={styles.source}>
                      <Popover>
                        <PopoverTrigger asChild>
                          <Button
                            variant="ghost"
                            size="sm"
                            className="w-full justify-start overflow-hidden"
                            disabled={!sourceKnown}
                            aria-label={`查看版本 r${commit.revision} 的来源`}
                          >
                            <span className="truncate">
                              {sourceKnown
                                ? `${short(commit.sourceWorkspace)} · ${sourceBaseOnPage ? `基线 r${sourceBase}` : `r${sourceBase} 页外基线`}`
                                : "来源不可用"}
                            </span>
                          </Button>
                        </PopoverTrigger>
                        <PopoverContent align="end" className="grid gap-2">
                          <p className="text-sm font-medium">来源工作区</p>
                          {source ? (
                            <Button
                              variant="link"
                              className="h-auto justify-start whitespace-normal break-all p-0 text-left"
                              onClick={() => chooseWorkspace(source)}
                            >
                              {commit.sourceWorkspace}
                            </Button>
                          ) : (
                            <span className="break-all text-sm">
                              {commit.sourceWorkspace}
                            </span>
                          )}
                          {sourceBase !== undefined && (
                            <span className="text-sm text-muted-foreground">
                              来源基线 r{sourceBase}
                            </span>
                          )}
                        </PopoverContent>
                      </Popover>
                    </div>
                  </li>
                );
              })}
            </ol>
          </div>
        </div>
      )}
      {selected && !selectedBaseOnPage && !busy && (
        <p className={styles.reference}>
          所选工作区基线 r{selected.baseRevision}（页外基线）
        </p>
      )}
      <footer className={styles.footer}>
        <span>
          <GitPullRequest aria-hidden="true" /> 本页 {rows.length} 个版本
        </span>
        <div className={styles.paging}>
          <Button
            variant="outline"
            size="sm"
            type="button"
            disabled={busy}
            onClick={onRefresh}
          >
            刷新
          </Button>
          <Button
            variant="outline"
            size="sm"
            type="button"
            disabled={busy || current.uppers.length === 1}
            onClick={() =>
              setHistory((state) => ({
                ...state,
                uppers: state.uppers.slice(0, -1),
              }))
            }
          >
            较新
          </Button>
          <Button
            variant="outline"
            size="sm"
            type="button"
            disabled={busy || !next}
            onClick={() =>
              setHistory((state) => ({
                ...state,
                uppers: [...state.uppers, next!],
              }))
            }
          >
            更早
          </Button>
        </div>
      </footer>
    </section>
  );
}

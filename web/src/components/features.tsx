"use client";
import {
  ApiError,
  call,
  type Dataset,
  type Feature,
  type FeaturePage,
  type Project,
  type Workspace,
} from "@/lib/browser-api";
import { featureText, pretty } from "@/lib/geojson";
import {
  buildLayer,
  drawKinds,
  formatBounds,
  kindLabels,
  matchesQuery,
  pageContaining,
  pageOf,
  type DrawKind,
  type GeometryKind,
  type Layer,
  type LayerItem,
} from "@/lib/map";
import {
  publication,
  readPublication,
  releasePublication,
  type Publication,
} from "@/lib/publication";
import {
  ArrowLeft,
  ArrowRight,
  Check,
  ChevronLeft,
  ChevronRight,
  Crosshair,
  Database,
  Ellipsis,
  MapPinned,
  PanelLeft,
  PanelRight,
  Pencil,
  Plus,
  RefreshCw,
  Search,
  Table2,
  Trash2,
  TriangleAlert,
  X,
} from "lucide-react";
import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { ErrorBox, Modal } from "./common";
import { MapView, kindColors, type MapHandle } from "./map-view";
import { short, useAction } from "./resource-shared";
import { Button } from "./ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "./ui/dropdown-menu";
import { Input } from "./ui/input";
import { Textarea } from "./ui/textarea";

/** Features fetched per load; further batches load on demand. */
const BATCH = 1000;
/** Rows per list / attribute-table page. */
const PAGE = 100;
const CHANGED = "工作区已变化。请刷新后重新加载。";

export function FeatureExplorer({
  project,
  dataset,
  writable,
  back,
}: {
  project: Project;
  dataset: Dataset;
  writable: boolean;
  back: () => void;
}) {
  const [workspaces, setWorkspaces] = useState<Workspace[]>([]),
    [workspace, setWorkspace] = useState(""),
    [refresh, setRefresh] = useState(0),
    [lookup, setLookup] = useState(false);
  const [workspaceError, setWorkspaceError] = useState("");
  const task = useAction();
  useEffect(() => {
    let active = true;
    call<Workspace[]>({ action: "workspaces", project: project.id, limit: 100 })
      .then((rows) => {
        if (active)
          setWorkspaces((previous) =>
            rows.some((w) => w.id === workspace)
              ? rows
              : [...previous.filter((w) => w.id === workspace), ...rows],
          );
      })
      .catch((e) => {
        if (active) setWorkspaceError(e.message);
      });
    return () => {
      active = false;
    };
  }, [project.id, refresh, workspace]);
  const source = (
    <>
      <div className="gis-title">
        <Button
          variant="ghost"
          size="icon-sm"
          onClick={back}
          aria-label="返回数据集"
          title="返回数据集"
        >
          <ArrowLeft />
        </Button>
        <span className="row-icon is-data" aria-hidden="true">
          <Database size={15} />
        </span>
        <div className="feature-title">
          <h2>{dataset.name}</h2>
          <span className="mono" title={dataset.id}>
            {short(dataset.id)}
          </span>
        </div>
      </div>
      <div className="gis-source">
        <select
          id="workspace-select"
          aria-label="数据来源"
          value={workspace}
          onChange={(e) => setWorkspace(e.target.value)}
        >
          <option value="">已发布的数据</option>
          {workspaces
            .filter((w) => w.status === "open" || w.id === workspace)
            .map((w) => (
              <option key={w.id} value={w.id}>
                工作区 {short(w.id)} · v{w.version}
              </option>
            ))}
        </select>
        <Button
          variant="outline"
          size="sm"
          aria-label="新建工作区"
          title="新建工作区"
          disabled={!writable || task.busy}
          onClick={() =>
            void task.run(async () => {
              const w = await call<Workspace>({
                action: "createWorkspace",
                project: project.id,
              });
              setWorkspaces((rows) => [w, ...rows]);
              setWorkspace(w.id);
              setRefresh((n) => n + 1);
            })
          }
        >
          <Plus />
          <span className="btn-label">新建工作区</span>
        </Button>
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label="更多来源"
              title="更多来源"
            >
              <Ellipsis />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="menu">
            <DropdownMenuItem onSelect={() => setLookup(true)}>
              <Search />
              使用指定工作区
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
    </>
  );
  return (
    <section className="gis-workspace" aria-label={`${dataset.name} 地图`}>
      <FeatureWorkspace
        key={`${dataset.id}:${workspace}`}
        project={project}
        dataset={dataset}
        workspaceId={workspace}
        writable={writable}
        source={source}
        alert={<ErrorBox message={task.error || workspaceError} />}
      />
      {lookup && (
        <Modal title="使用指定工作区" close={() => setLookup(false)}>
          <form
            className="lookup"
            onSubmit={(e) => {
              e.preventDefault();
              const id = String(
                new FormData(e.currentTarget).get("workspace"),
              ).trim();
              void task.run(async () => {
                const w = await call<Workspace>({
                  action: "workspace",
                  project: project.id,
                  workspace: id,
                });
                setWorkspaces((rows) => [
                  w,
                  ...rows.filter((x) => x.id !== w.id),
                ]);
                setWorkspace(w.id);
                setLookup(false);
              });
            }}
          >
            <Input
              name="workspace"
              aria-label="指定工作区标识"
              required
              autoFocus
              placeholder="工作区标识"
            />
            <Button variant="outline" disabled={task.busy}>
              使用工作区
            </Button>
          </form>
          <ErrorBox message={task.error} />
        </Modal>
      )}
    </section>
  );
}

type LayerState = {
  features: Feature[];
  revision?: string;
  version?: string;
  status: string;
  next?: string;
  busy: boolean;
  loaded: number;
  error: string;
};

/**
 * Loads a dataset view in pages of 100, pinned to one revision (published) or one
 * workspace version, up to BATCH features per call of `load`.
 */
function useLayer(project: string, dataset: string, workspaceId: string) {
  const [state, setState] = useState<LayerState>({
    features: [],
    status: "open",
    busy: true,
    loaded: 0,
    error: "",
  });
  const run = useRef(0);
  const cursor = useRef<{
    after?: string;
    revision?: string;
    version?: string;
  }>({});
  const load = useCallback(
    async (append: boolean) => {
      const mine = ++run.current;
      const start = append ? cursor.current : {};
      const base = append ? state.features.length : 0;
      setState((s) => ({
        ...s,
        busy: true,
        error: "",
        ...(append ? {} : { loaded: 0 }),
      }));
      try {
        const rows: Feature[] = [];
        let { after, revision, version } = start;
        do {
          const page = await call<FeaturePage>({
            action: "features",
            project,
            dataset,
            workspace: workspaceId || undefined,
            revision: !workspaceId ? revision : undefined,
            after,
            limit: 100,
          });
          if (run.current !== mine) return;
          if (workspaceId) {
            if (version !== undefined && page.workspaceVersion !== version)
              throw new Error(CHANGED);
            version = page.workspaceVersion;
          } else revision ??= page.revision;
          rows.push(...page.features);
          after = page.nextAfter;
          setState((s) => ({ ...s, loaded: base + rows.length }));
        } while (after && rows.length < BATCH);
        let status = "open";
        if (workspaceId) {
          const w = await call<Workspace>({
            action: "workspace",
            project,
            workspace: workspaceId,
          });
          if (w.version !== version) throw new Error(CHANGED);
          status = w.status;
        }
        if (run.current !== mine) return;
        cursor.current = { after, revision, version };
        setState((s) => ({
          features: append ? [...s.features, ...rows] : rows,
          revision,
          version,
          status,
          next: after,
          busy: false,
          loaded: base + rows.length,
          error: "",
        }));
      } catch (e) {
        if (run.current === mine)
          setState((s) => ({
            ...s,
            busy: false,
            error: e instanceof Error ? e.message : "加载失败",
          }));
      }
    },
    [project, dataset, workspaceId, state.features.length],
  );
  return { ...state, load };
}

function useNarrow() {
  const [narrow, setNarrow] = useState(false);
  useEffect(() => {
    const query = window.matchMedia("(max-width: 1024px)");
    const update = () => setNarrow(query.matches);
    update();
    query.addEventListener("change", update);
    return () => query.removeEventListener("change", update);
  }, []);
  return narrow;
}

function leaveFullscreen() {
  if (document.fullscreenElement) void document.exitFullscreen();
}

export function FeatureWorkspace({
  project,
  dataset,
  workspaceId,
  writable,
  source,
  alert,
}: {
  project: Project;
  dataset: Dataset;
  workspaceId: string;
  writable: boolean;
  source: ReactNode;
  alert: ReactNode;
}) {
  const layerState = useLayer(project.id, dataset.id, workspaceId);
  const { load } = layerState;
  const [refresh, setRefresh] = useState(0);
  const [selectedId, setSelectedId] = useState<string>(),
    [hovered, setHovered] = useState<number>(),
    [query, setQuery] = useState(""),
    [page, setPage] = useState(0),
    [visible, setVisible] = useState<Record<DrawKind, boolean>>({
      point: true,
      line: true,
      polygon: true,
    });
  const [edit, setEdit] = useState<Feature | "new">(),
    [remove, setRemove] = useState<Feature>(),
    [publish, setPublish] = useState(false);
  const [pending, setPending] = useState<Publication>();
  const narrow = useNarrow();
  const [left, setLeft] = useState(true),
    [right, setRight] = useState(true),
    [table, setTable] = useState(false),
    [tableHeight, setTableHeight] = useState(240),
    [fullscreen, setFullscreen] = useState(false);
  const body = useRef<HTMLDivElement>(null);
  const mapRef = useRef<MapHandle>(null);
  const task = useAction();

  useEffect(() => {
    setPending(readPublication(sessionStorage.getItem("gl.publication")));
  }, []);
  useEffect(() => {
    void load(false);
    // `load` changes with the loaded length; reload only on explicit refresh.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [refresh]);
  useEffect(() => {
    setLeft(!narrow);
    setRight(!narrow);
  }, [narrow]);
  useEffect(() => {
    const update = () =>
      setFullscreen(document.fullscreenElement === body.current);
    document.addEventListener("fullscreenchange", update);
    return () => document.removeEventListener("fullscreenchange", update);
  }, []);

  const layer = useMemo(
    () => buildLayer(layerState.features),
    [layerState.features],
  );
  const filtered = useMemo(
    () => layer.items.filter((item) => matchesQuery(item, query)),
    [layer, query],
  );
  const collection = useMemo(() => {
    if (!query.trim()) return layer.collection;
    const keep = new Set(filtered.map((item) => item.index));
    return {
      ...layer.collection,
      features: layer.collection.features.filter((f) =>
        keep.has(f.id as number),
      ),
    };
  }, [layer, filtered, query]);
  const paged = pageOf(filtered, page, PAGE);
  const selected = selectedId
    ? layer.items.find((item) => item.id === selectedId)
    : undefined;
  const { version, revision, status } = layerState;
  const busy = layerState.busy;
  const canEdit =
    writable &&
    !!workspaceId &&
    status === "open" &&
    version !== undefined &&
    !pending &&
    !busy &&
    !layerState.error;

  const reload = () => setRefresh((n) => n + 1);
  const select = (item: LayerItem | undefined, from: "map" | "list") => {
    setSelectedId(item?.id);
    if (!item) return;
    if (from === "list" && item.bounds) mapRef.current?.reveal(item.bounds);
    if (from === "map") {
      const at = pageContaining(filtered, (row) => row.id === item.id, PAGE);
      if (at !== undefined) setPage(at);
    }
    if (narrow) {
      setLeft(false);
      setRight(true);
    }
  };
  const describe = (index: number) => {
    const item = layer.items[index];
    return item ? (item.label ? `${item.id} · ${item.label}` : item.id) : "";
  };
  const raw = (item: LayerItem): Feature => ({
    id: item.id,
    geojson: item.raw,
  });
  const toggleFullscreen = () => {
    if (document.fullscreenElement) void document.exitFullscreen();
    else void body.current?.requestFullscreen?.();
  };
  const startResize = (event: React.PointerEvent) => {
    const startY = event.clientY;
    const startHeight = tableHeight;
    const max = (body.current?.clientHeight ?? 600) - 160;
    const move = (e: PointerEvent) =>
      setTableHeight(
        Math.round(
          Math.min(max, Math.max(120, startHeight + startY - e.clientY)),
        ),
      );
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  };

  return (
    <>
      <header className="gis-toolbar">
        {source}
        <span
          className="revision-tag"
          title={workspaceId ? "工作区版本" : "发布版本"}
        >
          {workspaceId ? `v${version ?? "…"}` : `r${revision ?? "…"}`}
        </span>
        <div className="gis-actions">
          <Button
            variant="outline"
            size="sm"
            aria-label="添加要素"
            title="添加要素"
            disabled={!canEdit}
            onClick={() => {
              leaveFullscreen();
              setEdit("new");
            }}
          >
            <Plus />
            <span className="btn-label">添加要素</span>
          </Button>
          {workspaceId && (
            <Button
              size="sm"
              disabled={
                !writable ||
                (status !== "open" && !pending) ||
                version === undefined ||
                busy
              }
              onClick={() => {
                leaveFullscreen();
                setPublish(true);
              }}
            >
              发布版本
              <ArrowRight />
            </Button>
          )}
        </div>
        <span className="gis-break" aria-hidden="true" />
        <div className="gis-toggles" role="group" aria-label="面板">
          <Button
            variant="ghost"
            size="icon-sm"
            aria-pressed={left}
            aria-label="图层与要素"
            title="图层与要素"
            onClick={() => {
              setLeft((v) => !v);
              if (narrow) setRight(false);
            }}
          >
            <PanelLeft />
          </Button>
          <Button
            variant="ghost"
            size="icon-sm"
            aria-pressed={table}
            aria-label="属性表"
            title="属性表"
            onClick={() => setTable((v) => !v)}
          >
            <Table2 />
          </Button>
          <Button
            variant="ghost"
            size="icon-sm"
            aria-pressed={right}
            aria-label="要素信息"
            title="要素信息"
            onClick={() => {
              setRight((v) => !v);
              if (narrow) setLeft(false);
            }}
          >
            <PanelRight />
          </Button>
        </div>
      </header>
      <div className="gis-alerts">
        {alert}
        <ErrorBox message={layerState.error || task.error} />
        {pending && (
          <div className="notice is-warning">
            <TriangleAlert aria-hidden="true" />
            <span>
              有一笔待确认的发布请求。请到对应工作区重试原请求，再继续编辑。
            </span>
          </div>
        )}
      </div>
      <div
        ref={body}
        className={`gis-body${left ? " has-left" : ""}${right ? " has-right" : ""}`}
      >
        <aside
          className="gis-panel gis-left"
          aria-label="图层与要素"
          hidden={!left}
        >
          <LayerPanel
            name={dataset.name}
            layer={layer}
            visible={visible}
            setVisible={setVisible}
            query={query}
            setQuery={(q) => {
              setQuery(q);
              setPage(0);
            }}
            rows={paged.rows}
            total={filtered.length}
            page={paged.page}
            pages={paged.pages}
            setPage={setPage}
            selectedId={selectedId}
            hovered={hovered}
            setHovered={setHovered}
            select={(item) => select(item, "list")}
            busy={busy}
            more={!!layerState.next}
            loadMore={() => void load(true)}
            reload={reload}
            close={narrow ? () => setLeft(false) : undefined}
          />
        </aside>
        <div className="gis-center">
          <MapView
            ref={mapRef}
            collection={collection}
            bounds={layer.bounds}
            fitKey={layer.bounds ? "data" : ""}
            selected={selected?.bounds ? selected.index : undefined}
            hovered={hovered}
            visible={visible}
            onSelect={(index) =>
              select(
                index === undefined ? undefined : layer.items[index],
                "map",
              )
            }
            onHover={setHovered}
            describe={describe}
            fullscreen={fullscreen}
            toggleFullscreen={toggleFullscreen}
          >
            {busy && (
              <div className="map-progress" role="status">
                <RefreshCw className="spin" aria-hidden="true" />
                正在加载 {layerState.loaded} 项
              </div>
            )}
            {!busy && !layerState.error && !layer.items.length && (
              <div className="map-empty">
                <MapPinned aria-hidden="true" />
                <strong>暂无要素</strong>
              </div>
            )}
          </MapView>
          {table && (
            <section
              className="gis-table"
              style={{ height: tableHeight }}
              aria-label="属性表"
            >
              <div
                className="gis-resize"
                role="separator"
                aria-orientation="horizontal"
                aria-label="调整属性表高度"
                onPointerDown={startResize}
              />
              <AttributeTable
                columns={layer.columns}
                rows={paged.rows}
                selectedId={selectedId}
                hovered={hovered}
                setHovered={setHovered}
                select={(item) => select(item, "list")}
                summary={`${filtered.length} 项 · 第 ${paged.page + 1}/${paged.pages} 页`}
                close={() => setTable(false)}
              />
            </section>
          )}
        </div>
        <aside
          className="gis-panel gis-right"
          aria-label="要素信息"
          hidden={!right}
        >
          {selected ? (
            <Inspector
              item={selected}
              canEdit={canEdit}
              zoom={() =>
                selected.bounds && mapRef.current?.fit(selected.bounds)
              }
              edit={() => {
                leaveFullscreen();
                setEdit(raw(selected));
              }}
              remove={() => {
                leaveFullscreen();
                setRemove(raw(selected));
              }}
              close={() => setSelectedId(undefined)}
            />
          ) : (
            <LayerSummary
              name={dataset.name}
              layer={layer}
              source={
                workspaceId
                  ? `工作区 v${version ?? "…"}`
                  : `r${revision ?? "…"}`
              }
              close={narrow ? () => setRight(false) : undefined}
            />
          )}
        </aside>
        {narrow && (left || right) && (
          <button
            className="gis-backdrop"
            aria-label="关闭面板"
            onClick={() => {
              setLeft(false);
              setRight(false);
            }}
          />
        )}
      </div>
      {edit && (
        <FeatureEditor
          feature={edit}
          close={() => setEdit(undefined)}
          save={async (id, text) => {
            await call({
              action: "save",
              project: project.id,
              workspace: workspaceId,
              version,
              edits: [{ dataset: dataset.id, featureId: id, feature: text }],
            });
            setEdit(undefined);
            setSelectedId(id);
            reload();
          }}
        />
      )}
      {remove && (
        <Modal
          title="删除要素"
          description={`将在当前工作区中删除 ${remove.id}。发布前不会改变正式版本。`}
          close={task.busy ? () => {} : () => setRemove(undefined)}
        >
          <ErrorBox message={task.error} />
          <div className="form-actions">
            <Button
              variant="outline"
              onClick={() => setRemove(undefined)}
              disabled={task.busy}
            >
              取消
            </Button>
            <Button
              variant="destructive"
              disabled={task.busy}
              onClick={() =>
                void task.run(async () => {
                  await call({
                    action: "save",
                    project: project.id,
                    workspace: workspaceId,
                    version,
                    edits: [
                      {
                        dataset: dataset.id,
                        featureId: remove.id,
                        feature: null,
                      },
                    ],
                  });
                  setRemove(undefined);
                  setSelectedId(undefined);
                  reload();
                })
              }
            >
              确认删除
            </Button>
          </div>
        </Modal>
      )}
      {publish && (
        <PublishDialog
          project={project.id}
          workspace={workspaceId}
          version={version!}
          close={() => {
            setPublish(false);
            setPending(
              readPublication(sessionStorage.getItem("gl.publication")),
            );
          }}
          complete={() => {
            setPending(undefined);
            reload();
          }}
        />
      )}
    </>
  );
}

function KindIcon({ item }: { item: Pick<LayerItem, "kind" | "draws"> }) {
  const kind = item.kind;
  if (kind === "none" || kind === "invalid")
    return (
      <span
        className={`kind-icon is-${kind}`}
        title={kindLabels[kind]}
        aria-hidden="true"
      />
    );
  const draw = kind === "mixed" ? (item.draws[0] ?? "point") : kind;
  return <Swatch kind={draw} mixed={kind === "mixed"} />;
}

function Swatch({ kind, mixed }: { kind: DrawKind; mixed?: boolean }) {
  return (
    <span
      className={`kind-icon is-${kind}${mixed ? " is-mixed" : ""}`}
      style={{ "--kind": kindColors[kind] } as React.CSSProperties}
      aria-hidden="true"
    />
  );
}

function LayerPanel({
  name,
  layer,
  visible,
  setVisible,
  query,
  setQuery,
  rows,
  total,
  page,
  pages,
  setPage,
  selectedId,
  hovered,
  setHovered,
  select,
  busy,
  more,
  loadMore,
  reload,
  close,
}: {
  name: string;
  layer: Layer;
  visible: Record<DrawKind, boolean>;
  setVisible: (v: Record<DrawKind, boolean>) => void;
  query: string;
  setQuery: (q: string) => void;
  rows: LayerItem[];
  total: number;
  page: number;
  pages: number;
  setPage: (p: number) => void;
  selectedId?: string;
  hovered?: number;
  setHovered: (i?: number) => void;
  select: (item: LayerItem) => void;
  busy: boolean;
  more: boolean;
  loadMore: () => void;
  reload: () => void;
  close?: () => void;
}) {
  const list = useRef<HTMLUListElement>(null);
  useEffect(() => {
    if (!selectedId) return;
    list.current
      ?.querySelector<HTMLElement>(`[data-id="${CSS.escape(selectedId)}"]`)
      ?.scrollIntoView({ block: "nearest" });
  }, [selectedId, page]);
  const present = drawKinds.filter(
    (k) =>
      layer.counts[k] > 0 ||
      (layer.counts.mixed > 0 &&
        layer.items.some((i) => i.kind === "mixed" && i.draws.includes(k))),
  );
  const all = present.length > 0 && present.every((k) => visible[k]);
  return (
    <>
      <div className="gis-panel-head">
        <strong>图层</strong>
        {close && (
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label="关闭"
            onClick={close}
          >
            <X />
          </Button>
        )}
      </div>
      <div className="layer-tree">
        <label className="layer-row">
          <input
            type="checkbox"
            checked={all}
            disabled={!present.length}
            onChange={(e) =>
              setVisible({
                point: e.target.checked,
                line: e.target.checked,
                polygon: e.target.checked,
              })
            }
          />
          <Database size={14} aria-hidden="true" />
          <span className="layer-name">{name}</span>
          <span className="layer-count">{layer.items.length}</span>
        </label>
        {present.map((kind) => (
          <label className="layer-row is-child" key={kind}>
            <input
              type="checkbox"
              checked={visible[kind]}
              onChange={(e) =>
                setVisible({ ...visible, [kind]: e.target.checked })
              }
            />
            <Swatch kind={kind} />
            <span className="layer-name">{kindLabels[kind]}</span>
            <span className="layer-count">{layer.counts[kind] || ""}</span>
          </label>
        ))}
      </div>
      <div className="gis-panel-head is-sub">
        <strong>要素</strong>
        <Button
          variant="ghost"
          size="icon-sm"
          onClick={reload}
          disabled={busy}
          aria-label="刷新"
          title="刷新"
        >
          <RefreshCw className={busy ? "spin" : undefined} />
        </Button>
      </div>
      <div className="search gis-search">
        <Search size={15} aria-hidden="true" />
        <Input
          type="search"
          aria-label="搜索要素"
          placeholder="搜索标识或属性"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
      </div>
      <ul className="feature-list" ref={list} aria-label="要素列表">
        {rows.map((item) => (
          <li key={item.id}>
            <button
              type="button"
              data-id={item.id}
              className={`feature-row${item.id === selectedId ? " is-selected" : ""}${item.index === hovered ? " is-hovered" : ""}`}
              aria-current={item.id === selectedId ? "true" : undefined}
              onClick={() => select(item)}
              onMouseEnter={() =>
                setHovered(item.bounds ? item.index : undefined)
              }
              onMouseLeave={() => setHovered(undefined)}
            >
              <KindIcon item={item} />
              <span className="feature-row-text">
                <span className="mono">{item.id}</span>
                {item.label && <small>{item.label}</small>}
              </span>
              {(item.kind === "none" || item.kind === "invalid") && (
                <small className="feature-row-flag">
                  {kindLabels[item.kind]}
                </small>
              )}
            </button>
          </li>
        ))}
        {!busy && !rows.length && (
          <li className="feature-list-empty">
            {query.trim() ? "没有匹配的要素" : "暂无要素"}
          </li>
        )}
      </ul>
      <div className="gis-panel-foot">
        <span>
          {query.trim()
            ? `${total} / ${layer.items.length} 项`
            : more
              ? `已加载 ${layer.items.length} 项`
              : `${total} 项`}
        </span>
        {more && (
          <Button variant="ghost" size="sm" disabled={busy} onClick={loadMore}>
            继续加载
          </Button>
        )}
        {pages > 1 && (
          <div className="gis-pager">
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label="上一页"
              disabled={page === 0}
              onClick={() => setPage(page - 1)}
            >
              <ChevronLeft />
            </Button>
            <span className="mono">
              {page + 1}/{pages}
            </span>
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label="下一页"
              disabled={page >= pages - 1}
              onClick={() => setPage(page + 1)}
            >
              <ChevronRight />
            </Button>
          </div>
        )}
      </div>
    </>
  );
}

function Inspector({
  item,
  canEdit,
  zoom,
  edit,
  remove,
  close,
}: {
  item: LayerItem;
  canEdit: boolean;
  zoom: () => void;
  edit: () => void;
  remove: () => void;
  close: () => void;
}) {
  return (
    <>
      <div className="gis-panel-head">
        <KindIcon item={item} />
        <strong className="mono inspector-id" title={item.id}>
          {item.id}
        </strong>
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label="取消选择"
          onClick={close}
        >
          <X />
        </Button>
      </div>
      <div className="inspector-actions">
        <Button
          variant="outline"
          size="sm"
          disabled={!item.bounds}
          onClick={zoom}
        >
          <Crosshair />
          缩放至要素
        </Button>
        <Button variant="outline" size="sm" disabled={!canEdit} onClick={edit}>
          <Pencil />
          编辑
        </Button>
        <Button
          variant="outline"
          size="sm"
          className="is-danger"
          disabled={!canEdit}
          onClick={remove}
        >
          <Trash2 />
          删除
        </Button>
      </div>
      <div className="inspector-body">
        <section>
          <h3>
            属性 <span>{item.properties.length}</span>
          </h3>
          {item.properties.length ? (
            <dl className="attr-list">
              {item.properties.map((p) => (
                <div key={p.key}>
                  <dt title={p.key}>{p.key}</dt>
                  <dd
                    className={p.kind === "string" ? undefined : `is-${p.kind}`}
                  >
                    {p.value}
                  </dd>
                </div>
              ))}
            </dl>
          ) : (
            <p className="muted">无属性</p>
          )}
        </section>
        <section>
          <h3>几何</h3>
          <dl className="attr-list">
            <div>
              <dt>类型</dt>
              <dd>{item.type}</dd>
            </div>
            {item.error ? (
              <div>
                <dt>状态</dt>
                <dd className="is-error">{item.error}</dd>
              </div>
            ) : (
              item.bounds && (
                <>
                  <div>
                    <dt>部件</dt>
                    <dd className="is-number">{item.parts}</dd>
                  </div>
                  <div>
                    <dt>顶点</dt>
                    <dd className="is-number">{item.vertices}</dd>
                  </div>
                  <div>
                    <dt>范围</dt>
                    <dd className="is-number">{formatBounds(item.bounds)}</dd>
                  </div>
                </>
              )
            )}
          </dl>
        </section>
        <details className="inspector-raw">
          <summary>GeoJSON</summary>
          <pre className="json-view">{pretty(item.raw)}</pre>
        </details>
      </div>
    </>
  );
}

function LayerSummary({
  name,
  layer,
  source,
  close,
}: {
  name: string;
  layer: Layer;
  source: string;
  close?: () => void;
}) {
  const kinds = (Object.keys(layer.counts) as GeometryKind[]).filter(
    (k) => layer.counts[k] > 0,
  );
  return (
    <>
      <div className="gis-panel-head">
        <strong>图层信息</strong>
        {close && (
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label="关闭"
            onClick={close}
          >
            <X />
          </Button>
        )}
      </div>
      <div className="inspector-body">
        <dl className="attr-list">
          <div>
            <dt>图层</dt>
            <dd>{name}</dd>
          </div>
          <div>
            <dt>来源</dt>
            <dd className="is-number">{source}</dd>
          </div>
          <div>
            <dt>要素</dt>
            <dd className="is-number">{layer.items.length}</dd>
          </div>
          {kinds.map((k) => (
            <div key={k}>
              <dt>{kindLabels[k]}</dt>
              <dd className="is-number">{layer.counts[k]}</dd>
            </div>
          ))}
          <div>
            <dt>坐标系</dt>
            <dd>WGS 84 (EPSG:4326)</dd>
          </div>
          {layer.bounds && (
            <div>
              <dt>范围</dt>
              <dd className="is-number">{formatBounds(layer.bounds)}</dd>
            </div>
          )}
        </dl>
        <div className="inspector-hint">
          <MapPinned aria-hidden="true" />
          未选择要素
        </div>
      </div>
    </>
  );
}

function AttributeTable({
  columns,
  rows,
  selectedId,
  hovered,
  setHovered,
  select,
  summary,
  close,
}: {
  columns: string[];
  rows: LayerItem[];
  selectedId?: string;
  hovered?: number;
  setHovered: (i?: number) => void;
  select: (item: LayerItem) => void;
  summary: string;
  close: () => void;
}) {
  const body = useRef<HTMLTableSectionElement>(null);
  useEffect(() => {
    if (!selectedId) return;
    body.current
      ?.querySelector<HTMLElement>(`[data-id="${CSS.escape(selectedId)}"]`)
      ?.scrollIntoView({ block: "nearest" });
  }, [selectedId, rows]);
  return (
    <>
      <div className="gis-table-head">
        <strong>属性表</strong>
        <span>{summary}</span>
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label="关闭属性表"
          onClick={close}
        >
          <X />
        </Button>
      </div>
      <div className="gis-table-scroll">
        <table>
          <thead>
            <tr>
              <th>要素标识</th>
              <th>几何</th>
              {columns.map((c) => (
                <th key={c} title={c}>
                  {c}
                </th>
              ))}
            </tr>
          </thead>
          <tbody ref={body}>
            {rows.map((item) => {
              const values = new Map(item.properties.map((p) => [p.key, p]));
              return (
                <tr
                  key={item.id}
                  data-id={item.id}
                  className={`${item.id === selectedId ? "is-selected" : ""}${item.index === hovered ? " is-hovered" : ""}`}
                  aria-selected={item.id === selectedId}
                  onClick={() => select(item)}
                  onMouseEnter={() =>
                    setHovered(item.bounds ? item.index : undefined)
                  }
                  onMouseLeave={() => setHovered(undefined)}
                >
                  <td className="mono">{item.id}</td>
                  <td>{kindLabels[item.kind]}</td>
                  {columns.map((c) => {
                    const v = values.get(c);
                    return (
                      <td
                        key={c}
                        className={
                          v && v.kind !== "string" ? "mono" : undefined
                        }
                        title={v?.value}
                      >
                        {v?.value ?? ""}
                      </td>
                    );
                  })}
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    </>
  );
}
export function FeatureEditor({
  feature,
  close,
  save,
}: {
  feature: Feature | "new";
  close: () => void;
  save: (id: string, raw: string) => Promise<void>;
}) {
  const task = useAction();
  return (
    <Modal
      title={feature === "new" ? "添加要素" : "编辑要素"}
      description="输入 GeoJSON Feature，发布后生效。"
      close={task.busy ? () => {} : close}
    >
      <form
        onSubmit={(e) => {
          e.preventDefault();
          const form = new FormData(e.currentTarget);
          const id =
            feature === "new" ? String(form.get("id")).trim() : feature.id;
          void task.run(() =>
            save(id, featureText(String(form.get("geojson")), id)),
          );
        }}
      >
        <label htmlFor="feature-id">要素标识</label>
        <Input
          id="feature-id"
          name="id"
          required
          maxLength={256}
          defaultValue={feature === "new" ? "" : feature.id}
          readOnly={feature !== "new"}
        />
        <label htmlFor="geojson">GeoJSON</label>
        <Textarea
          className="code-editor"
          id="geojson"
          name="geojson"
          required
          rows={14}
          spellCheck={false}
          defaultValue={
            feature === "new"
              ? '{\n  "type": "Feature",\n  "properties": {},\n  "geometry": {\n    "type": "Point",\n    "coordinates": [104, 35]\n  }\n}'
              : pretty(feature.geojson)
          }
        />
        <ErrorBox message={task.error} />
        <div className="form-actions">
          <Button
            type="button"
            variant="outline"
            onClick={close}
            disabled={task.busy}
          >
            取消
          </Button>
          <Button type="submit" disabled={task.busy}>
            {task.busy ? "正在保存…" : "保存到工作区"}
          </Button>
        </div>
      </form>
    </Modal>
  );
}
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
  const [result, setResult] = useState<string>();
  const task = useAction();
  const matches =
    !pending ||
    (pending.project === project && pending.workspace === workspace);
  return (
    <Modal
      title={result ? "发布完成" : "发布新版本"}
      description="合并工作区更改；有冲突时需先解决。"
      close={task.busy ? () => {} : close}
    >
      {result ? (
        <div className="publish-success">
          <Check />
          <h3>版本 r{result} 已发布</h3>
          <Button onClick={close}>完成</Button>
        </div>
      ) : (
        <form
          onSubmit={(e) => {
            e.preventDefault();
            void task.run(async () => {
              const intent =
                pending ??
                publication(project, workspace, version, message.trim());
              // Persist before sending: an interrupted response can only retry this immutable request.
              sessionStorage.setItem("gl.publication", JSON.stringify(intent));
              setPending(intent);
              try {
                const response = await call<{ revision: string }>(intent);
                sessionStorage.removeItem("gl.publication");
                setPending(undefined);
                setResult(response.revision);
                complete();
              } catch (error) {
                if (
                  error instanceof ApiError &&
                  releasePublication(error.status, error.uncertain, !!pending)
                ) {
                  sessionStorage.removeItem("gl.publication");
                  setPending(undefined);
                }
                throw error;
              }
            });
          }}
        >
          <label htmlFor="publish-message">版本说明</label>
          <Textarea
            id="publish-message"
            required
            maxLength={2048}
            value={message}
            readOnly={!!pending}
            onChange={(e) => setMessage(e.target.value)}
            placeholder="例如：更新道路边界与分类"
          />
          {pending && (
            <div className="notice">
              {matches
                ? "保留了原始发布请求，重试会使用相同内容和请求标识。"
                : `请先到工作区 ${pending.workspace} 确认上一笔发布。`}
            </div>
          )}
          <ErrorBox message={task.error} />
          <div className="form-actions">
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
              disabled={task.busy || !matches || !message.trim()}
            >
              {task.busy ? "正在发布…" : pending ? "重试原发布" : "确认发布"}
            </Button>
          </div>
        </form>
      )}
    </Modal>
  );
}

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
  importBatch,
  assertGeometryType,
  geometryLabels,
  type ImportedFeature,
} from "@/lib/geojson-import";
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
  ChevronsUpDown,
  Crosshair,
  Database,
  GitBranch,
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
import { Confirm, ErrorBox, Modal, Notice, Tip, useMedia } from "./common";
import { MapView, kindColors, type MapHandle } from "./map-view";
import { short, useAction } from "./resource-shared";
import { Badge } from "./ui/badge";
import { Button } from "./ui/button";
import { Checkbox } from "./ui/checkbox";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "./ui/collapsible";
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from "./ui/command";
import { Input } from "./ui/input";
import { Label } from "./ui/label";
import { Popover, PopoverContent, PopoverTrigger } from "./ui/popover";
import {
  ResizableHandle,
  ResizablePanel,
  ResizablePanelGroup,
} from "./ui/resizable";
import { ScrollArea, ScrollBar } from "./ui/scroll-area";
import { Separator } from "./ui/separator";
import { Sheet, SheetContent, SheetTitle } from "./ui/sheet";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "./ui/table";
import { Textarea } from "./ui/textarea";
import { Toggle } from "./ui/toggle";

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
    [refresh, setRefresh] = useState(0);
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
        <Tip label="返回数据集">
          <Button
            variant="ghost"
            size="icon-sm"
            onClick={back}
            aria-label="返回数据集"
          >
            <ArrowLeft />
          </Button>
        </Tip>
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
      <Separator orientation="vertical" className="gis-sep" />
      <div className="gis-source">
        <SourcePicker
          value={workspace}
          workspaces={workspaces.filter(
            (w) => w.status === "open" || w.id === workspace,
          )}
          disabled={task.busy}
          choose={setWorkspace}
          lookup={(id) =>
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
            })
          }
        />
        <Tip label="新建工作区">
          <Button
            variant="outline"
            size="sm"
            aria-label="新建工作区"
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
        </Tip>
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
    </section>
  );
}

/**
 * Data source combobox: the published view, open workspaces, or any workspace
 * looked up by its full identifier (for workspaces beyond the first page).
 */
function SourcePicker({
  value,
  workspaces,
  disabled,
  choose,
  lookup,
}: {
  value: string;
  workspaces: Workspace[];
  disabled: boolean;
  choose: (id: string) => void;
  lookup: (id: string) => void;
}) {
  const [open, setOpen] = useState(false),
    [search, setSearch] = useState("");
  const current = workspaces.find((w) => w.id === value);
  const typed = search.trim();
  const known = !typed || workspaces.some((w) => w.id === typed);
  const toggle = (next: boolean) => {
    setOpen(next);
    if (!next) setSearch("");
  };
  const pick = (id: string) => {
    toggle(false);
    if (id !== value) choose(id);
  };
  return (
    <Popover open={open} onOpenChange={toggle}>
      <PopoverTrigger asChild>
        <Button
          variant="outline"
          size="sm"
          role="combobox"
          aria-expanded={open}
          aria-label="数据来源"
          data-value={value}
          disabled={disabled}
          className="source-trigger"
        >
          {value ? <GitBranch /> : <Database />}
          <span className="source-label">
            {value
              ? `工作区 ${short(value)}${current ? ` · v${current.version}` : ""}`
              : "已发布的数据"}
          </span>
          <ChevronsUpDown className="source-chevron" />
        </Button>
      </PopoverTrigger>
      <PopoverContent align="start" className="source-menu">
        <Command>
          <CommandInput
            aria-label="工作区标识"
            placeholder="搜索或输入工作区标识"
            value={search}
            onValueChange={setSearch}
          />
          <CommandList>
            <CommandEmpty>没有匹配的来源</CommandEmpty>
            <CommandGroup>
              <CommandItem
                value="已发布的数据 published"
                onSelect={() => pick("")}
              >
                <Database />
                <span className="source-item-label">已发布的数据</span>
                <Check className={value ? "invisible" : undefined} />
              </CommandItem>
            </CommandGroup>
            {workspaces.length > 0 && (
              <CommandGroup heading="工作区">
                {workspaces.map((w) => (
                  <CommandItem
                    key={w.id}
                    value={`${w.id} v${w.version}`}
                    onSelect={() => pick(w.id)}
                  >
                    <GitBranch />
                    <span className="source-item-label">
                      <span className="mono">{short(w.id)}</span>
                      <small>v{w.version}</small>
                    </span>
                    <Check
                      className={w.id === value ? undefined : "invisible"}
                    />
                  </CommandItem>
                ))}
              </CommandGroup>
            )}
            {!known && (
              <CommandGroup forceMount>
                <CommandItem
                  forceMount
                  value={`lookup ${typed}`}
                  onSelect={() => {
                    toggle(false);
                    lookup(typed);
                  }}
                >
                  <Search />
                  <span className="source-item-label">
                    使用工作区 <span className="mono">{typed}</span>
                  </span>
                </CommandItem>
              </CommandGroup>
            )}
          </CommandList>
        </Command>
      </PopoverContent>
    </Popover>
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
  const [topologyWarnings, setTopologyWarnings] = useState<{
    count: number;
    samples: string[];
  }>({ count: 0, samples: [] });
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
  // Wide screens dock both panels; narrow screens show one of them as a sheet.
  const narrow = useMedia("(max-width: 1024px)");
  const phone = useMedia("(max-width: 640px)");
  const [left, setLeft] = useState(true),
    [right, setRight] = useState(true),
    [drawer, setDrawer] = useState<"left" | "right">(),
    [table, setTable] = useState(false),
    [fullscreen, setFullscreen] = useState(false);
  const [body, setBody] = useState<HTMLDivElement | null>(null);
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
    setDrawer(undefined);
  }, [narrow]);
  useEffect(() => {
    const update = () =>
      setFullscreen(!!body && document.fullscreenElement === body);
    document.addEventListener("fullscreenchange", update);
    return () => document.removeEventListener("fullscreenchange", update);
  }, [body]);

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
    if (narrow) setDrawer("right");
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
    else void body?.requestFullscreen?.();
  };
  const showLeft = narrow ? drawer === "left" : left;
  const showRight = narrow ? drawer === "right" : right;
  const closeDrawer = narrow ? () => setDrawer(undefined) : undefined;

  const leftPanel = (
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
      close={closeDrawer}
    />
  );
  const rightPanel = selected ? (
    <Inspector
      item={selected}
      canEdit={canEdit}
      zoom={() => selected.bounds && mapRef.current?.fit(selected.bounds)}
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
      source={workspaceId ? `工作区 v${version ?? "…"}` : `r${revision ?? "…"}`}
      close={closeDrawer}
    />
  );
  return (
    <>
      <header className="gis-toolbar">
        {source}
        <Badge
          variant="outline"
          className="revision-tag"
          title={workspaceId ? "工作区版本" : "发布版本"}
        >
          {workspaceId ? `v${version ?? "…"}` : `r${revision ?? "…"}`}
        </Badge>
        <div className="gis-actions">
          <Tip label="添加要素">
            <Button
              variant="outline"
              size="sm"
              aria-label="添加要素"
              disabled={!canEdit}
              onClick={() => {
                leaveFullscreen();
                setEdit("new");
              }}
            >
              <Plus />
              <span className="btn-label">添加要素</span>
            </Button>
          </Tip>
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
        <Separator orientation="vertical" className="gis-sep is-toggles" />
        <div className="gis-toggles" role="group" aria-label="面板">
          <Tip label="图层与要素">
            <Toggle
              size="sm"
              pressed={showLeft}
              aria-label="图层与要素"
              onPressedChange={(on) =>
                narrow ? setDrawer(on ? "left" : undefined) : setLeft(on)
              }
            >
              <PanelLeft />
            </Toggle>
          </Tip>
          <Tip label="属性表">
            <Toggle
              size="sm"
              pressed={table}
              aria-label="属性表"
              onPressedChange={setTable}
            >
              <Table2 />
            </Toggle>
          </Tip>
          <Tip label="要素信息">
            <Toggle
              size="sm"
              pressed={showRight}
              aria-label="要素信息"
              onPressedChange={(on) =>
                narrow ? setDrawer(on ? "right" : undefined) : setRight(on)
              }
            >
              <PanelRight />
            </Toggle>
          </Tip>
        </div>
      </header>
      <div className="gis-alerts">
        {alert}
        {topologyWarnings.count > 0 && (
          <Notice tone="warning" role="status">
            已原样保存。发现 {topologyWarnings.count}{" "}
            个要素存在拓扑问题（如自相交），未自动修复坐标。
            <ul>
              {topologyWarnings.samples.map((warning, index) => (
                <li key={index}>{warning}</li>
              ))}
            </ul>
          </Notice>
        )}

        <ErrorBox message={layerState.error || task.error} />
        {pending && (
          <Notice tone="warning" icon={<TriangleAlert aria-hidden="true" />}>
            有一笔待确认的发布请求。请到对应工作区重试原请求，再继续编辑。
          </Notice>
        )}
      </div>
      <div ref={setBody} className="gis-body">
        {!narrow && (
          <aside
            className="gis-panel gis-left"
            aria-label="图层与要素"
            hidden={!left}
          >
            {leftPanel}
          </aside>
        )}
        <div className="gis-center">
          <ResizablePanelGroup orientation="vertical" className="gis-split">
            <ResizablePanel id="map" minSize={160} className="gis-map-panel">
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
            </ResizablePanel>
            {table && (
              <>
                <ResizableHandle
                  className="gis-split-handle"
                  aria-label="调整属性表高度"
                />
                <ResizablePanel
                  id="table"
                  defaultSize={240}
                  minSize={120}
                  maxSize="70%"
                  className="gis-table-panel"
                >
                  <section className="gis-table" aria-label="属性表">
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
                </ResizablePanel>
              </>
            )}
          </ResizablePanelGroup>
        </div>
        {!narrow && (
          <aside
            className="gis-panel gis-right"
            aria-label="要素信息"
            hidden={!right}
          >
            {rightPanel}
          </aside>
        )}
        {narrow && (
          <>
            <Sheet
              open={drawer === "left"}
              onOpenChange={(open) => !open && setDrawer(undefined)}
            >
              <SheetContent
                side="left"
                container={body}
                showCloseButton={false}
                aria-describedby={undefined}
                className="gis-panel gis-left"
              >
                <SheetTitle className="sr-only">图层与要素</SheetTitle>
                {leftPanel}
              </SheetContent>
            </Sheet>
            <Sheet
              open={drawer === "right"}
              onOpenChange={(open) => !open && setDrawer(undefined)}
            >
              <SheetContent
                side={phone ? "bottom" : "right"}
                container={body}
                showCloseButton={false}
                aria-describedby={undefined}
                className="gis-panel gis-right"
              >
                <SheetTitle className="sr-only">要素信息</SheetTitle>
                {rightPanel}
              </SheetContent>
            </Sheet>
          </>
        )}
      </div>
      {edit && (
        <FeatureEditor
          geometryType={dataset.geometryType}
          feature={edit}
          close={() => {
            setEdit(undefined);
            reload();
          }}
          save={async (features, nextVersion) => {
            const result = await call<{ version: string; warnings?: string[] }>(
              {
                action: "save",
                project: project.id,
                workspace: workspaceId,
                version: nextVersion ?? version,
                edits: features.map(({ id, raw }) => ({
                  dataset: dataset.id,
                  featureId: id,
                  feature: raw,
                })),
              },
            );
            if (result.warnings?.length) {
              setTopologyWarnings((previous) => ({
                count: previous.count + result.warnings!.length,
                samples: [...previous.samples, ...result.warnings!].slice(0, 5),
              }));
            }
            setSelectedId(features[0].id);
            return result.version;
          }}
        />
      )}
      {remove && (
        <Confirm
          title="删除要素"
          description={`将在当前工作区中删除 ${remove.id}。发布前不会改变正式版本。`}
          action="确认删除"
          busy={task.busy}
          error={task.error}
          close={() => setRemove(undefined)}
          confirm={() =>
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
        />
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
        <Label className="layer-row">
          <Checkbox
            checked={all}
            disabled={!present.length}
            onCheckedChange={(checked) => {
              const on = checked === true;
              setVisible({ point: on, line: on, polygon: on });
            }}
          />
          <Database size={14} aria-hidden="true" />
          <span className="layer-name">{name}</span>
          <span className="layer-count">{layer.items.length}</span>
        </Label>
        {present.map((kind) => (
          <Label className="layer-row is-child" key={kind}>
            <Checkbox
              checked={visible[kind]}
              onCheckedChange={(checked) =>
                setVisible({ ...visible, [kind]: checked === true })
              }
            />
            <Swatch kind={kind} />
            <span className="layer-name">{kindLabels[kind]}</span>
            <span className="layer-count">{layer.counts[kind] || ""}</span>
          </Label>
        ))}
      </div>
      <div className="gis-panel-head is-sub">
        <strong>要素</strong>
        <Tip label="刷新">
          <Button
            variant="ghost"
            size="icon-sm"
            onClick={reload}
            disabled={busy}
            aria-label="刷新"
          >
            <RefreshCw className={busy ? "spin" : undefined} />
          </Button>
        </Tip>
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
      <ScrollArea className="feature-scroll">
        <ul className="feature-list" ref={list} aria-label="要素列表">
          {rows.map((item) => (
            <li key={item.id}>
              <Button
                variant="ghost"
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
              </Button>
            </li>
          ))}
          {!busy && !rows.length && (
            <li className="feature-list-empty">
              {query.trim() ? "没有匹配的要素" : "暂无要素"}
            </li>
          )}
        </ul>
      </ScrollArea>
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
      <ScrollArea className="inspector-scroll">
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
                      className={
                        p.kind === "string" ? undefined : `is-${p.kind}`
                      }
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
          <Collapsible className="inspector-raw">
            <CollapsibleTrigger asChild>
              <Button
                variant="ghost"
                size="xs"
                className="inspector-raw-trigger"
              >
                <ChevronRight />
                GeoJSON
              </Button>
            </CollapsibleTrigger>
            <CollapsibleContent>
              <pre className="json-view">{pretty(item.raw)}</pre>
            </CollapsibleContent>
          </Collapsible>
        </div>
      </ScrollArea>
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
      <ScrollArea className="inspector-scroll">
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
      </ScrollArea>
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
      <ScrollArea className="gis-table-scroll">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>要素标识</TableHead>
              <TableHead>几何</TableHead>
              {columns.map((c) => (
                <TableHead key={c} title={c}>
                  {c}
                </TableHead>
              ))}
            </TableRow>
          </TableHeader>
          <TableBody ref={body}>
            {rows.map((item) => {
              const values = new Map(item.properties.map((p) => [p.key, p]));
              return (
                <TableRow
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
                  <TableCell className="mono">{item.id}</TableCell>
                  <TableCell>{kindLabels[item.kind]}</TableCell>
                  {columns.map((c) => {
                    const v = values.get(c);
                    return (
                      <TableCell
                        key={c}
                        className={
                          v && v.kind !== "string" ? "mono" : undefined
                        }
                        title={v?.value}
                      >
                        {v?.value ?? ""}
                      </TableCell>
                    );
                  })}
                </TableRow>
              );
            })}
          </TableBody>
        </Table>
        <ScrollBar orientation="horizontal" />
      </ScrollArea>
    </>
  );
}
export function FeatureEditor({
  geometryType,
  feature,
  close,
  save,
}: {
  geometryType: Dataset["geometryType"];
  feature: Feature | "new";
  close: () => void;
  save: (features: ImportedFeature[], version?: string) => Promise<string>;
}) {
  const task = useAction();
  const [saved, setSaved] = useState(0);
  const cursor = useRef<{
    rows?: ImportedFeature[];
    offset: number;
    version?: string;
  }>({ offset: 0 });
  async function submit(rows: ImportedFeature[]) {
    if (cursor.current.rows !== rows)
      cursor.current = { rows, offset: 0, version: cursor.current.version };
    const state = cursor.current;
    while (state.offset < rows.length) {
      const batch = importBatch(rows, state.offset);
      state.version = await save(batch, state.version);
      state.offset += batch.length;
      setSaved(state.offset);
    }
    close();
  }
  const [upload, setUpload] = useState<{
    name: string;
    features: ImportedFeature[];
  }>();
  return (
    <Modal
      title={feature === "new" ? "添加要素" : "编辑要素"}
      description="输入 GeoJSON Feature 或上传文件，保存到工作区后发布生效。"
      close={task.busy ? () => {} : close}
    >
      <form
        onSubmit={(e) => {
          e.preventDefault();
          if (upload) {
            void task.run(() => submit(upload.features));
            return;
          }
          const form = new FormData(e.currentTarget);
          const id =
            feature === "new" ? String(form.get("id")).trim() : feature.id;
          void task.run(() =>
            (async () => {
              const raw = featureText(String(form.get("geojson")), id);
              assertGeometryType(JSON.parse(raw).geometry, geometryType);
              await submit([{ id, raw }]);
            })(),
          );
        }}
      >
        {feature === "new" && (
          <>
            <Label htmlFor="geojson-file">上传 GeoJSON 文件</Label>
            <Input
              id="geojson-file"
              type="file"
              accept=".geojson,.json,application/geo+json,application/json"
              disabled={task.busy}
              onChange={(event) => {
                const file = event.target.files?.[0];
                setUpload(undefined);
                setSaved(0);
                if (!file) return;
                void task.run(async () => {
                  const features = await new Promise<ImportedFeature[]>(
                    (resolve, reject) => {
                      const worker = new Worker(
                        new URL(
                          "../lib/geojson-import.worker.ts",
                          import.meta.url,
                        ),
                      );
                      worker.onmessage = (
                        event: MessageEvent<{
                          rows?: ImportedFeature[];
                          error?: string;
                        }>,
                      ) => {
                        worker.terminate();
                        if (event.data.error)
                          reject(new Error(event.data.error));
                        else resolve(event.data.rows!);
                      };
                      worker.onerror = () => {
                        worker.terminate();
                        reject(new Error("GeoJSON 解析失败，请重试。"));
                      };
                      worker.postMessage({ file, family: geometryType });
                    },
                  );
                  setUpload({ name: file.name, features });
                });
              }}
            />
            <p className="muted">
              当前数据集类型：{geometryLabels[geometryType]}。支持
              Feature、FeatureCollection 和几何对象，不限制文件大小。 缺少 id
              时自动生成，已有同名要素将被更新。
            </p>
            {task.busy && !upload && (
              <p role="status">正在解析 GeoJSON 文件…</p>
            )}
            {upload && (
              <p role="status">
                已读取 {upload.name}，共 {upload.features.length}{" "}
                个要素，保存后分批导入工作区。
                {upload.features.some((f) => f.reprojected) &&
                  " 已将 EPSG:3857 米制坐标转换为 WGS84 经纬度。"}
              </p>
            )}
          </>
        )}
        <fieldset disabled={task.busy || !!upload} hidden={!!upload}>
          <Label htmlFor="feature-id">要素标识</Label>
          <Input
            id="feature-id"
            name="id"
            required
            maxLength={256}
            defaultValue={feature === "new" ? "" : feature.id}
            readOnly={feature !== "new"}
          />
          <Label htmlFor="geojson">GeoJSON</Label>
          <Textarea
            className="code-editor"
            id="geojson"
            name="geojson"
            required
            rows={14}
            spellCheck={false}
            defaultValue={
              feature === "new"
                ? JSON.stringify(
                    {
                      type: "Feature",
                      properties: {},
                      geometry: {
                        point: { type: "Point", coordinates: [104, 35] },
                        line: {
                          type: "LineString",
                          coordinates: [
                            [104, 35],
                            [105, 36],
                          ],
                        },
                        polygon: {
                          type: "Polygon",
                          coordinates: [
                            [
                              [104, 35],
                              [105, 35],
                              [105, 36],
                              [104, 35],
                            ],
                          ],
                        },
                      }[geometryType],
                    },
                    null,
                    2,
                  )
                : pretty(feature.geojson)
            }
          />
        </fieldset>
        {saved > 0 && (
          <p role="status">
            已保存 {saved}{" "}
            个要素。中断时已完成的批次会保留，重试从未完成批次继续；若提示版本冲突，请关闭并刷新后检查工作区。
          </p>
        )}
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
          <Label htmlFor="publish-message">版本说明</Label>
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
            <Notice>
              {matches
                ? "保留了原始发布请求，重试会使用相同内容和请求标识。"
                : `请先到工作区 ${pending.workspace} 确认上一笔发布。`}
            </Notice>
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

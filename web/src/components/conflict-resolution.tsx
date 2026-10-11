"use client";

import {
  ApiError,
  call,
  type Conflicts,
  type Workspace,
} from "@/lib/browser-api";
import { featureName } from "@/lib/changes";
import {
  attributeRows,
  choiceKind,
  conflictKind,
  isWholeFeatureConflict,
  parseConflict,
  parseLosslessJson,
  resolutionOf,
  resultSlot,
  sideSelection,
  type AttributeRow,
  type ChoiceKind,
  type Conflict,
  type ConflictKind,
  type ConflictSide,
  type Selection,
  type Slot,
} from "@/lib/conflicts";
import { count, valueText } from "@/lib/format";
import { featureText } from "@/lib/geojson";
import { readPublication } from "@/lib/publication";
import { cn } from "@/lib/utils";
import { stringify } from "lossless-json";
import {
  ArrowRight,
  Check,
  ChevronDown,
  CircleCheck,
  CircleDashed,
  GitMerge,
  ListChecks,
  Pencil,
  RefreshCw,
  Search,
  TriangleAlert,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { toast } from "sonner";
import {
  CopyButton,
  Empty,
  ErrorBox,
  FooterStart,
  ListSkeleton,
  Modal,
  Notice,
} from "./common";
import styles from "./conflict-resolution.module.css";
import { GeometryPreview } from "./geometry-preview";
import { short } from "./resource-shared";
import { useDatasetNames } from "./use-datasets";
import { Badge } from "./ui/badge";
import { Button } from "./ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "./ui/dropdown-menu";
import { Input } from "./ui/input";
import { Kbd } from "./ui/kbd";
import { Progress } from "./ui/progress";
import { Skeleton } from "./ui/skeleton";
import { Textarea } from "./ui/textarea";
import { ToggleGroup, ToggleGroupItem } from "./ui/toggle-group";

type LoadedConflict = Conflict & { cursor: string };
type ResolutionResult = {
  version: string;
  head: string;
  remainingConflicts: string;
};
type RebaseResult = { baseRevision: string; version: string };
const PAGE = 100;

function conflictKey(conflict: LoadedConflict): string {
  return `${conflict.dataset}\u0000${conflict.featureId}`;
}
const kindText: Record<ConflictKind, string> = {
  stale: "需重新确认",
  "deleted-draft": "我已删除",
  "deleted-current": "已发布删除",
  geometry: "几何",
  attributes: "属性",
  both: "属性与几何",
  feature: "整个要素",
};
const choiceText: Record<ChoiceKind, string> = {
  draft: "我的",
  current: "已发布",
  mixed: "混合",
  custom: "自定义",
};
function isTyping(target: EventTarget | null) {
  const element = target as HTMLElement | null;
  return (
    !!element &&
    (element.tagName === "INPUT" ||
      element.tagName === "TEXTAREA" ||
      element.isContentEditable)
  );
}

export function Resolution({
  project,
  workspace,
  mode,
  close,
  complete,
  readOnly = false,
}: {
  project: string;
  workspace: Workspace;
  mode: "resolve" | "rebase";
  close: () => void;
  complete: (result?: { revision?: string }) => void;
  readOnly?: boolean;
}) {
  const datasetName = useDatasetNames(project);
  const [data, setData] = useState<Conflicts>();
  const [items, setItems] = useState<LoadedConflict[]>([]);
  const [selectedKey, setSelectedKey] = useState<string>();
  const [choices, setChoices] = useState<Record<string, Selection>>({});
  const [filter, setFilter] = useState<"all" | "open" | "done">("all");
  const [query, setQuery] = useState("");
  const [busy, setBusy] = useState(false);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [loadError, setLoadError] = useState("");
  const [reload, setReload] = useState(0);
  const [reconfirm, setReconfirm] = useState(false);
  const [editing, setEditing] = useState(false);
  const [editor, setEditor] = useState("");
  const [editorError, setEditorError] = useState("");
  const [showAll, setShowAll] = useState(false);
  const snapshot = useRef<string | undefined>(undefined);
  const listRef = useRef<HTMLElement>(null);

  const load = useCallback(async () => {
    setLoading(true);
    setLoadError("");
    try {
      const result = await call<Conflicts>({
        action: "conflicts",
        project,
        workspace: workspace.id,
        limit: PAGE,
      });
      const identity = `${result.version}:${result.head}`;
      if (snapshot.current && snapshot.current !== identity) setChoices({});
      snapshot.current = identity;
      const parsed = result.conflicts.map((entry) => ({
        ...parseConflict(entry.json),
        cursor: entry.cursor,
      }));
      setData(result);
      setItems(parsed);
      setSelectedKey((key) =>
        parsed.some((item) => conflictKey(item) === key)
          ? key
          : parsed[0] && conflictKey(parsed[0]),
      );
    } catch (cause) {
      setLoadError(cause instanceof Error ? cause.message : "无法加载冲突。");
    } finally {
      setLoading(false);
    }
  }, [project, workspace.id]);

  useEffect(() => {
    void load();
  }, [load, reload]);

  const resolvedCount = items.filter(
    (item) => resolutionOf(item, choices[conflictKey(item)]) !== undefined,
  ).length;
  const visible = useMemo(() => {
    const needle = query.trim().toLowerCase();
    return items.filter((item) => {
      const done = resolutionOf(item, choices[conflictKey(item)]) !== undefined;
      if (filter === "open" && done) return false;
      if (filter === "done" && !done) return false;
      if (!needle) return true;
      const name =
        featureName(item.draft) ??
        featureName(item.current) ??
        featureName(item.base) ??
        "";
      return [item.featureId, name, datasetName(item.dataset)].some((text) =>
        text.toLowerCase().includes(needle),
      );
    });
  }, [items, choices, filter, query, datasetName]);
  const current =
    items.find((item) => conflictKey(item) === selectedKey) ?? items[0];
  const currentKey = current && conflictKey(current);
  const selection = currentKey ? choices[currentKey] : undefined;
  const result = current ? resolutionOf(current, selection) : undefined;

  useEffect(() => {
    setEditing(false);
    setEditorError("");
  }, [currentKey]);

  const setChoice = (key: string, next: Selection | undefined) => {
    setReconfirm(false);
    setChoices((previous) => {
      const copy = { ...previous };
      if (next) copy[key] = next;
      else delete copy[key];
      return copy;
    });
  };
  const chooseSide = (side: ConflictSide) => {
    if (!current || busy || readOnly) return;
    setEditing(false);
    setChoice(currentKey!, sideSelection(current, side));
  };
  const chooseField = (pointer: string, side: ConflictSide) => {
    if (!current || busy || readOnly) return;
    setEditing(false);
    if (isWholeFeatureConflict(current)) {
      setChoice(currentKey!, { fields: { "*": side } });
      return;
    }
    const prior = choices[currentKey!]?.fields ?? {};
    const { ["*"]: _whole, ...fields } = prior;
    void _whole;
    setChoice(currentKey!, { fields: { ...fields, [pointer]: side } });
  };
  const bulk = (side: ConflictSide | undefined) => {
    if (busy || readOnly) return;
    setEditing(false);
    setChoices(
      side
        ? Object.fromEntries(
            items.map((item) => [conflictKey(item), sideSelection(item, side)]),
          )
        : {},
    );
  };
  const move = (delta: number) => {
    if (!visible.length) return;
    const index = visible.findIndex((item) => conflictKey(item) === currentKey);
    const next =
      visible[Math.min(visible.length - 1, Math.max(0, index + delta))];
    setSelectedKey(conflictKey(next));
    requestAnimationFrame(() =>
      listRef.current
        ?.querySelector<HTMLElement>('[aria-current="true"]')
        ?.scrollIntoView({ block: "nearest" }),
    );
  };
  const nextOpen = items.find(
    (item) =>
      conflictKey(item) !== currentKey &&
      resolutionOf(item, choices[conflictKey(item)]) === undefined,
  );
  const startEdit = () => {
    if (!current) return;
    const fallback = current.draft ?? current.current ?? current.base;
    const seed = result ?? (fallback ? stringify(fallback) : null);
    setEditor(seed ? stringify(parseLosslessJson(seed), undefined, 2)! : "");
    setEditorError("");
    setEditing(true);
  };
  const applyEdit = () => {
    if (!current) return;
    try {
      const text = featureText(editor, current.featureId);
      setChoice(currentKey!, { fields: {}, text });
      setEditing(false);
      setEditorError("");
    } catch (cause) {
      setEditorError(cause instanceof Error ? cause.message : "GeoJSON 无效。");
    }
  };

  const edits = items.flatMap((item) => {
    const text = resolutionOf(item, choices[conflictKey(item)]);
    return text === undefined
      ? []
      : [{ dataset: item.dataset, featureId: item.featureId, feature: text }];
  });
  const total = data ? Number(data.total) : 0;
  const allLoaded = !!data && !data.truncated && items.length === total;
  const pageResolved = items.length > 0 && resolvedCount === items.length;
  const upToDate = !!data && data.head === workspace.baseRevision;
  const canSubmit =
    !readOnly &&
    !busy &&
    !loading &&
    !editing &&
    !!data &&
    (mode === "rebase" ? total === 0 || pageResolved : pageResolved);
  const finalStep = mode === "rebase" ? allLoaded : total <= items.length;

  const submit = async () => {
    if (!data || !canSubmit) return;
    if (readPublication(sessionStorage.getItem("gl.publication"))) {
      setError("请先确认待处理的发布请求。");
      return;
    }
    setBusy(true);
    setError("");
    try {
      if (mode === "rebase" && allLoaded) {
        const rebased = await call<RebaseResult>({
          action: "rebase",
          project,
          workspace: workspace.id,
          version: data.version,
          head: data.head,
          edits,
        });
        toast.success(`已更新到 r${rebased.baseRevision}`);
        complete({ revision: rebased.baseRevision });
        return;
      }
      const resolved = await call<ResolutionResult>({
        action: "resolve",
        project,
        workspace: workspace.id,
        version: data.version,
        head: data.head,
        edits,
      });
      if (mode === "resolve" && resolved.remainingConflicts === "0") {
        toast.success(`已解决 ${count(edits.length)} 个冲突`);
        complete();
        return;
      }
      toast.success(
        `已保存 ${count(edits.length)} 个，剩余 ${count(resolved.remainingConflicts)} 个`,
      );
      setChoices({});
      setReload((value) => value + 1);
    } catch (cause) {
      const message = cause instanceof Error ? cause.message : "保存失败。";
      setError(message);
      if (
        cause instanceof ApiError &&
        (cause.status === 409 || cause.uncertain)
      ) {
        setReconfirm(true);
        setChoices({});
        setReload((value) => value + 1);
      }
    } finally {
      setBusy(false);
    }
  };

  const keys = useRef<(event: globalThis.KeyboardEvent) => void>(() => {});
  keys.current = (event) => {
    if (
      isTyping(event.target) ||
      event.altKey ||
      event.ctrlKey ||
      event.metaKey
    )
      return;
    if (
      (event.target as HTMLElement | null)?.closest?.(
        '[data-slot="toggle-group"], [role="tablist"]',
      )
    )
      return;
    // Menus and nested dialogs own the keyboard while open.
    if (document.querySelector('[role="menu"]')) return;
    const dialogs = document.querySelectorAll('[role="dialog"]');
    const own = (event.target as HTMLElement | null)?.closest(
      '[role="dialog"]',
    );
    if (own && dialogs[dialogs.length - 1] !== own) return;
    if (event.key === "ArrowDown" || event.key === "j") {
      event.preventDefault();
      move(1);
    } else if (event.key === "ArrowUp" || event.key === "k") {
      event.preventDefault();
      move(-1);
    } else if (event.key === "1") chooseSide("draft");
    else if (event.key === "2") chooseSide("current");
  };
  useEffect(() => {
    const listener = (event: globalThis.KeyboardEvent) => keys.current(event);
    window.addEventListener("keydown", listener);
    return () => window.removeEventListener("keydown", listener);
  }, []);

  const headRevision = data?.head ?? "…";
  const title = readOnly
    ? "查看冲突"
    : mode === "rebase"
      ? "更新并解决冲突"
      : "解决冲突";
  const primaryLabel =
    mode === "rebase"
      ? finalStep
        ? `更新到 r${headRevision}`
        : `保存本批 ${count(edits.length)} 个`
      : finalStep
        ? `应用 ${count(edits.length)} 个解决`
        : `保存本批 ${count(edits.length)} 个`;
  const progress = items.length ? (resolvedCount / items.length) * 100 : 0;

  return (
    <Modal
      title={title}
      size="xl"
      busy={busy}
      close={close}
      className={styles.dialog}
      bodyClassName={styles.body}
      meta={
        <>
          <Badge variant="outline" className="meta-chip">
            <span className="mono">{short(workspace.id)}</span>
          </Badge>
          <Badge variant="outline" className="meta-chip">
            基线 r{workspace.baseRevision}
            <ArrowRight aria-hidden="true" />
            已发布 r{headRevision}
          </Badge>
        </>
      }
      footer={
        <>
          <FooterStart>
            {items.length > 0 && (
              <div className={styles.progress} role="status">
                <Progress
                  value={progress}
                  aria-label="解决进度"
                  className={styles.progressBar}
                />
                <span>
                  <strong>
                    {count(resolvedCount)}/{count(items.length)}
                  </strong>{" "}
                  已解决
                  {total > items.length && (
                    <span className="muted"> · 共 {count(total)}</span>
                  )}
                </span>
              </div>
            )}
          </FooterStart>
          <Button variant="outline" disabled={busy} onClick={close}>
            {readOnly ? "关闭" : "取消"}
          </Button>
          {!readOnly && (
            <Button disabled={!canSubmit} onClick={() => void submit()}>
              {busy ? <RefreshCw className="spin" /> : <Check />}
              {mode === "rebase" && total === 0
                ? `更新到 r${headRevision}`
                : primaryLabel}
            </Button>
          )}
        </>
      }
    >
      <div className={styles.frame}>
        <ErrorBox message={error} />
        {reconfirm && (
          <Notice tone="warning" icon={<TriangleAlert aria-hidden="true" />}>
            冲突已变化，已重新加载
          </Notice>
        )}
        {loading && !data ? (
          <div className={styles.layout}>
            <div className={styles.listPane}>
              <ListSkeleton rows={5} />
            </div>
            <div className={styles.detail}>
              <Skeleton className="h-6 w-56" />
              <Skeleton className="h-40 w-full" />
              <Skeleton className="h-32 w-full" />
            </div>
          </div>
        ) : loadError && !data ? (
          <ErrorBox message={loadError} retry={() => setReload((n) => n + 1)} />
        ) : !current ? (
          <Empty
            icon={CircleCheck}
            title={
              mode === "rebase" ? (upToDate ? "已是最新" : "无冲突") : "无冲突"
            }
          />
        ) : (
          <div className={styles.layout}>
            <div className={styles.listPane}>
              <div className={styles.listTools}>
                <div className="search">
                  <Search size={15} aria-hidden="true" />
                  <Input
                    value={query}
                    onChange={(event) => setQuery(event.target.value)}
                    placeholder="搜索要素"
                    aria-label="搜索冲突要素"
                  />
                </div>
                <div className={styles.listFilters}>
                  <ToggleGroup
                    type="single"
                    size="sm"
                    variant="outline"
                    value={filter}
                    onValueChange={(value) =>
                      value && setFilter(value as typeof filter)
                    }
                    aria-label="按状态筛选"
                    className="filter-group"
                  >
                    <ToggleGroupItem value="all">
                      全部
                      <span className="filter-count">
                        {count(items.length)}
                      </span>
                    </ToggleGroupItem>
                    <ToggleGroupItem value="open">
                      待解决
                      <span className="filter-count">
                        {count(items.length - resolvedCount)}
                      </span>
                    </ToggleGroupItem>
                    <ToggleGroupItem value="done">
                      已解决
                      <span className="filter-count">
                        {count(resolvedCount)}
                      </span>
                    </ToggleGroupItem>
                  </ToggleGroup>
                  {!readOnly && (
                    <DropdownMenu>
                      <DropdownMenuTrigger asChild>
                        <Button
                          variant="outline"
                          size="sm"
                          disabled={busy}
                          aria-label="批量操作"
                        >
                          <ListChecks />
                          <ChevronDown />
                        </Button>
                      </DropdownMenuTrigger>
                      <DropdownMenuContent align="end" className="menu">
                        <DropdownMenuItem onSelect={() => bulk("draft")}>
                          全部保留我的
                        </DropdownMenuItem>
                        <DropdownMenuItem onSelect={() => bulk("current")}>
                          全部采用已发布
                        </DropdownMenuItem>
                        <DropdownMenuSeparator />
                        <DropdownMenuItem
                          variant="destructive"
                          disabled={!resolvedCount}
                          onSelect={() => bulk(undefined)}
                        >
                          清除全部选择
                        </DropdownMenuItem>
                      </DropdownMenuContent>
                    </DropdownMenu>
                  )}
                </div>
              </div>
              <nav ref={listRef} className={styles.list} aria-label="冲突要素">
                {visible.map((item) => {
                  const key = conflictKey(item);
                  const kind = choiceKind(item, choices[key]);
                  const active = key === currentKey;
                  const name =
                    featureName(item.draft) ??
                    featureName(item.current) ??
                    featureName(item.base);
                  return (
                    <Button
                      key={item.cursor}
                      type="button"
                      variant="ghost"
                      aria-current={active ? "true" : undefined}
                      className={cn(styles.item, active && styles.itemActive)}
                      onClick={() => setSelectedKey(key)}
                    >
                      {kind ? (
                        <CircleCheck
                          className={styles.itemDone}
                          aria-label="已解决"
                        />
                      ) : (
                        <CircleDashed
                          className={styles.itemOpen}
                          aria-label="待解决"
                        />
                      )}
                      <span className={styles.itemText}>
                        <strong>{name ?? item.featureId}</strong>
                        <small>
                          {datasetName(item.dataset)}
                          {name && <> · {item.featureId}</>}
                        </small>
                      </span>
                      <span className={styles.itemTags}>
                        <Badge
                          variant="outline"
                          className={cn(
                            "kind-badge",
                            `is-conflict-${conflictKind(item)}`,
                          )}
                        >
                          {kindText[conflictKind(item)]}
                        </Badge>
                        {kind && (
                          <span className={styles.choiceTag}>
                            {choiceText[kind]}
                          </span>
                        )}
                      </span>
                    </Button>
                  );
                })}
                {!visible.length && <p className={styles.listEmpty}>无匹配</p>}
              </nav>
            </div>
            <section
              className={styles.detail}
              aria-label="冲突详情"
              aria-live="polite"
            >
              <ConflictDetail
                key={currentKey}
                conflict={current}
                dataset={datasetName(current.dataset)}
                selection={selection}
                result={result}
                base={workspace.baseRevision}
                head={headRevision}
                readOnly={readOnly}
                busy={busy}
                showAll={showAll}
                setShowAll={setShowAll}
                chooseSide={chooseSide}
                chooseField={chooseField}
                clear={() => setChoice(currentKey!, undefined)}
                editing={editing}
                startEdit={startEdit}
                cancelEdit={() => setEditing(false)}
                editor={editor}
                setEditor={(text) => {
                  setEditor(text);
                  setEditorError("");
                }}
                editorError={editorError}
                applyEdit={applyEdit}
                next={
                  nextOpen
                    ? () => setSelectedKey(conflictKey(nextOpen))
                    : undefined
                }
              />
            </section>
          </div>
        )}
      </div>
    </Modal>
  );
}

function ConflictDetail({
  conflict,
  dataset,
  selection,
  result,
  base,
  head,
  readOnly,
  busy,
  showAll,
  setShowAll,
  chooseSide,
  chooseField,
  clear,
  editing,
  startEdit,
  cancelEdit,
  editor,
  setEditor,
  editorError,
  applyEdit,
  next,
}: {
  conflict: LoadedConflict;
  dataset: string;
  selection: Selection | undefined;
  result: string | null | undefined;
  base: string;
  head: string;
  readOnly: boolean;
  busy: boolean;
  showAll: boolean;
  setShowAll: (value: boolean) => void;
  chooseSide: (side: ConflictSide) => void;
  chooseField: (pointer: string, side: ConflictSide) => void;
  clear: () => void;
  editing: boolean;
  startEdit: () => void;
  cancelEdit: () => void;
  editor: string;
  setEditor: (text: string) => void;
  editorError: string;
  applyEdit: () => void;
  next?: () => void;
}) {
  const rows = useMemo(() => attributeRows(conflict), [conflict]);
  const kind = conflictKind(conflict);
  const choice = choiceKind(conflict, selection);
  const name =
    featureName(conflict.draft) ??
    featureName(conflict.current) ??
    featureName(conflict.base);
  const whole = isWholeFeatureConflict(conflict);
  const shown = showAll ? rows : rows.filter((row) => row.changed);
  const hidden = rows.length - shown.length;
  const fieldChoice = (row: AttributeRow): ConflictSide | undefined =>
    selection?.text !== undefined
      ? undefined
      : (selection?.fields["*"] ?? selection?.fields[row.pointer]);
  const resultValue = (row: AttributeRow): Slot | undefined => {
    if (result === undefined) return;
    return resultSlot(result, row);
  };
  const columns: { side: ConflictSide; label: string }[] = [
    { side: "base", label: `基线 r${base}` },
    { side: "current", label: `已发布 r${head}` },
    { side: "draft", label: "我的" },
  ];
  return (
    <>
      <header className={styles.detailHeader}>
        <div className={styles.detailTitle}>
          <h3>{name ?? conflict.featureId}</h3>
          <div className={styles.detailMeta}>
            <span>{dataset}</span>
            <span className="mono">{conflict.featureId}</span>
            <CopyButton value={conflict.featureId} label="复制要素标识" />
            <Badge
              variant="outline"
              className={cn("kind-badge", `is-conflict-${kind}`)}
            >
              {kindText[kind]}
            </Badge>
            {kind === "stale" && conflict.resolvedAgainstRevision && (
              <Badge variant="outline" className="kind-badge is-deleted">
                原解决于 r{String(conflict.resolvedAgainstRevision)}
              </Badge>
            )}
          </div>
        </div>
        {!readOnly && (
          <div
            className={styles.sideActions}
            role="group"
            aria-label="解决方式"
          >
            <Button
              variant={choice === "draft" ? "default" : "outline"}
              size="sm"
              aria-pressed={choice === "draft"}
              disabled={busy}
              onClick={() => chooseSide("draft")}
            >
              保留我的
              <Kbd className={styles.kbd}>1</Kbd>
            </Button>
            <Button
              variant={choice === "current" ? "default" : "outline"}
              size="sm"
              aria-pressed={choice === "current"}
              disabled={busy}
              onClick={() => chooseSide("current")}
            >
              采用已发布
              <Kbd className={styles.kbd}>2</Kbd>
            </Button>
            <Button
              variant={choice === "custom" || editing ? "secondary" : "ghost"}
              size="sm"
              aria-pressed={editing}
              disabled={busy}
              onClick={editing ? cancelEdit : startEdit}
            >
              <Pencil />
              编辑
            </Button>
          </div>
        )}
      </header>
      {editing ? (
        <div className={styles.editor}>
          <Textarea
            aria-label="编辑解决结果 GeoJSON"
            className="code-editor"
            spellCheck={false}
            value={editor}
            onChange={(event) => setEditor(event.target.value)}
            autoFocus
          />
          <ErrorBox message={editorError} />
          <div className={styles.editorActions}>
            <Button variant="ghost" size="sm" onClick={cancelEdit}>
              取消
            </Button>
            <Button size="sm" onClick={applyEdit}>
              应用编辑
            </Button>
          </div>
        </div>
      ) : (
        <div className={styles.compare}>
          <div className={styles.tableWrap}>
            <table className={cn("diff-table", styles.table)}>
              <thead>
                <tr>
                  <th scope="col">字段</th>
                  {columns.map((column) => (
                    <th
                      scope="col"
                      key={column.side}
                      data-side={column.side}
                      className={styles[`col_${column.side}`]}
                    >
                      {column.label}
                    </th>
                  ))}
                  <th scope="col" className={styles.col_result}>
                    结果
                  </th>
                </tr>
              </thead>
              <tbody>
                {shown.map((row) => {
                  const picked = fieldChoice(row);
                  const value = resultValue(row);
                  const geometry = row.key === "\0geometry";
                  return (
                    <tr
                      key={row.key}
                      className={cn(
                        row.conflict && styles.rowConflict,
                        !row.conflict && row.changed && styles.rowChanged,
                      )}
                    >
                      <th scope="row">
                        {row.conflict && (
                          <GitMerge
                            className={styles.rowIcon}
                            aria-label="冲突"
                          />
                        )}
                        {row.label}
                      </th>
                      {columns.map(({ side }) => {
                        const slot = row[side];
                        const sideFeature = conflict[side];
                        const changed =
                          side !== "base" &&
                          !!sideFeature &&
                          (slot.present !== row.base.present ||
                            stringify(slot.value) !==
                              stringify(row.base.value));
                        const text = !sideFeature ? (
                          <span className="value-missing">已删除</span>
                        ) : slot.present ? (
                          valueText(true, slot.value, geometry)
                        ) : (
                          <span className="value-missing">—</span>
                        );
                        const selectable = row.conflict && !readOnly && !whole;
                        return (
                          <td
                            key={side}
                            data-side={side}
                            className={cn(
                              changed && styles.cellChanged,
                              picked === side && styles.cellPicked,
                            )}
                          >
                            {selectable ? (
                              <button
                                type="button"
                                className={styles.cellButton}
                                aria-pressed={picked === side}
                                aria-label={`${row.label} 选用${side === "draft" ? "我的" : side === "current" ? "已发布" : "基线"}`}
                                disabled={busy}
                                onClick={() => chooseField(row.pointer, side)}
                              >
                                <span>{text}</span>
                                {picked === side && (
                                  <Check aria-hidden="true" />
                                )}
                              </button>
                            ) : (
                              text
                            )}
                          </td>
                        );
                      })}
                      <td className={styles.resultCell}>
                        {result === null ? (
                          <span className="value-missing">已删除</span>
                        ) : value === undefined ? (
                          row.conflict ? (
                            <span className={styles.pending}>待选择</span>
                          ) : (
                            <span className="value-missing">自动</span>
                          )
                        ) : value.present ? (
                          valueText(true, value.value, geometry)
                        ) : (
                          <span className="value-missing">—</span>
                        )}
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
            {(hidden > 0 || showAll) && (
              <Button
                variant="ghost"
                size="xs"
                className={styles.toggleRows}
                onClick={() => setShowAll(!showAll)}
              >
                {showAll
                  ? "仅显示变更字段"
                  : `显示全部字段（+${count(hidden)}）`}
              </Button>
            )}
          </div>
          <GeometryPreview
            className={styles.preview}
            layers={[
              {
                id: "base",
                label: `基线`,
                geometry: conflict.base?.geometry,
                present: !!conflict.base,
                tone: "base",
              },
              {
                id: "current",
                label: `已发布`,
                geometry: conflict.current?.geometry,
                present: !!conflict.current,
                tone: "current",
              },
              {
                id: "draft",
                label: "我的",
                geometry: conflict.draft?.geometry,
                present: !!conflict.draft,
                tone: "draft",
              },
            ]}
          />
        </div>
      )}
      {!readOnly && !editing && (
        <footer className={styles.detailFooter}>
          <span className={styles.detailStatus}>
            {choice ? (
              <>
                <CircleCheck aria-hidden="true" />
                {choiceText[choice]}
              </>
            ) : (
              <>
                <CircleDashed aria-hidden="true" />
                待解决
              </>
            )}
          </span>
          {choice && (
            <Button variant="ghost" size="xs" onClick={clear} disabled={busy}>
              清除
            </Button>
          )}
          {next && (
            <Button
              variant="ghost"
              size="xs"
              className={styles.nextButton}
              onClick={next}
            >
              下一个待解决
              <ArrowRight />
            </Button>
          )}
        </footer>
      )}
    </>
  );
}

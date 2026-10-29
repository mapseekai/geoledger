"use client";
import { Table } from "./ui/table";
import { TableHeader } from "./ui/table";
import { TableRow } from "./ui/table";
import { TableHead } from "./ui/table";
import { TableBody } from "./ui/table";
import { TableCell } from "./ui/table";
import {
  diffRows,
  featureName,
  parseChange,
  type ChangeKind,
  type ParsedChange,
} from "@/lib/changes";
import { count, valueText } from "@/lib/format";
import { cn } from "@/lib/utils";
import { stringify } from "lossless-json";
import { Braces, ChevronRight, FileDiff } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { Empty, ErrorBox, ListSkeleton } from "./common";
import { GeometryPreview } from "./geometry-preview";
import { useDatasetNames } from "./use-datasets";
import { Badge } from "./ui/badge";
import { Button } from "./ui/button";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "./ui/collapsible";
import { ToggleGroup, ToggleGroupItem } from "./ui/toggle-group";

export const kindLabel: Record<ChangeKind, string> = {
  added: "新增",
  modified: "修改",
  deleted: "删除",
};
export function KindBadge({ kind }: { kind: ChangeKind }) {
  return (
    <Badge
      variant="outline"
      className={cn("kind-badge", {
        "is-added": kind === "added",
        "is-modified": kind === "modified",
        "is-deleted": kind === "deleted",
      })}
    >
      {kindLabel[kind]}
    </Badge>
  );
}
export type ChangePage = {
  rows: { cursor: string; json: string }[];
  next?: string;
};
/** Paged, filterable list of feature changes with per-feature attribute diffs. */
export function ChangeList({
  project,
  load,
  reloadKey,
  beforeLabel = "之前",
  afterLabel = "之后",
  className,
}: {
  project: string;
  load: (after: string) => Promise<ChangePage>;
  reloadKey?: string | number;
  beforeLabel?: string;
  afterLabel?: string;
  className?: string;
}) {
  const datasetName = useDatasetNames(project);
  const [rows, setRows] = useState<ParsedChange[]>([]);
  const [next, setNext] = useState<string>();
  const [busy, setBusy] = useState(true);
  const [error, setError] = useState("");
  const [filter, setFilter] = useState<"all" | ChangeKind>("all");
  const [retry, setRetry] = useState(0);
  const fetchPage = useCallback(
    async (after: string, append: boolean) => {
      setBusy(true);
      setError("");
      try {
        const page = await load(after);
        const parsed = page.rows.map((row) =>
          parseChange(row.cursor, row.json),
        );
        setRows((previous) => (append ? [...previous, ...parsed] : parsed));
        setNext(page.next);
      } catch (cause) {
        setError(cause instanceof Error ? cause.message : "加载失败");
      } finally {
        setBusy(false);
      }
    },
    // `load` is recreated by parents on every render; reloadKey drives refreshes.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [reloadKey],
  );
  useEffect(() => {
    // oxlint-disable-next-line typescript/no-floating-promises -- fetchPage catches request/parse failures and reports them in component state.
    void fetchPage("", false);
  }, [fetchPage, retry]);
  const counts = useMemo(() => {
    const result = { added: 0, modified: 0, deleted: 0 };
    for (const row of rows) result[row.kind]++;
    return result;
  }, [rows]);
  const visible =
    filter === "all" ? rows : rows.filter((r) => r.kind === filter);
  return (
    <div className={cn("change-list", className)}>
      {rows.length > 0 && (
        <ToggleGroup
          type="single"
          size="sm"
          variant="outline"
          value={filter}
          onValueChange={(value) => value && setFilter(value as typeof filter)}
          aria-label="按变更类型筛选"
          className="filter-group"
        >
          <ToggleGroupItem value="all">
            全部 <span className="filter-count">{count(rows.length)}</span>
          </ToggleGroupItem>
          {(["added", "modified", "deleted"] as const).map((kind) => (
            <ToggleGroupItem key={kind} value={kind} disabled={!counts[kind]}>
              {kindLabel[kind]}
              <span className="filter-count">{count(counts[kind])}</span>
            </ToggleGroupItem>
          ))}
        </ToggleGroup>
      )}
      <ErrorBox message={error} retry={() => setRetry((n) => n + 1)} />
      {busy && !rows.length ? (
        <ListSkeleton />
      ) : !rows.length && !error ? (
        <Empty icon={FileDiff} title="无要素变更" />
      ) : (
        <ul className="change-items" aria-label="要素变更">
          {visible.map((change) => (
            <ChangeItem
              key={change.cursor}
              change={change}
              dataset={datasetName(change.dataset)}
              beforeLabel={beforeLabel}
              afterLabel={afterLabel}
            />
          ))}
        </ul>
      )}
      {next && (
        <Button
          variant="outline"
          size="sm"
          className="load-more"
          disabled={busy}
          onClick={() => void fetchPage(next, true)}
        >
          {busy ? "加载中…" : "加载更多"}
        </Button>
      )}
    </div>
  );
}
function ChangeItem({
  change,
  dataset,
  beforeLabel,
  afterLabel,
}: {
  change: ParsedChange;
  dataset: string;
  beforeLabel: string;
  afterLabel: string;
}) {
  const [open, setOpen] = useState(false);
  const name = featureName(change.after) ?? featureName(change.before);
  return (
    <li>
      <Collapsible open={open} onOpenChange={setOpen}>
        <CollapsibleTrigger className="change-row">
          <ChevronRight className="change-chevron" aria-hidden="true" />
          <KindBadge kind={change.kind} />
          <span className="change-title">
            <strong>{name ?? change.featureId}</strong>
            {name && <span className="mono">{change.featureId}</span>}
          </span>
          <span className="change-dataset" title={change.dataset}>
            {dataset}
          </span>
        </CollapsibleTrigger>
        <CollapsibleContent>
          {open && (
            <ChangeDetail
              change={change}
              beforeLabel={beforeLabel}
              afterLabel={afterLabel}
            />
          )}
        </CollapsibleContent>
      </Collapsible>
    </li>
  );
}
function ChangeDetail({
  change,
  beforeLabel,
  afterLabel,
}: {
  change: ParsedChange;
  beforeLabel: string;
  afterLabel: string;
}) {
  const rows = diffRows(change);
  const geometry = rows[0];
  const [raw, setRaw] = useState(false);
  return (
    <div className="change-detail">
      <div className="change-detail-grid">
        <Table className="diff-table">
          <TableHeader>
            <TableRow>
              <TableHead scope="col">字段</TableHead>
              {change.kind !== "added" && (
                <TableHead scope="col">{beforeLabel}</TableHead>
              )}
              {change.kind !== "deleted" && (
                <TableHead scope="col">{afterLabel}</TableHead>
              )}
            </TableRow>
          </TableHeader>
          <TableBody>
            {rows.map((row) => (
              <TableRow key={row.key}>
                <TableHead scope="row">{row.label}</TableHead>
                {change.kind !== "added" && (
                  <TableCell
                    className={cn({
                      "is-old": row.changed && change.kind === "modified",
                    })}
                  >
                    {row.hasBefore ? (
                      valueText(true, row.before, row.geometry)
                    ) : (
                      <span className="value-missing">—</span>
                    )}
                  </TableCell>
                )}
                {change.kind !== "deleted" && (
                  <TableCell
                    className={cn({
                      "is-new": row.changed && change.kind === "modified",
                    })}
                  >
                    {row.hasAfter ? (
                      valueText(true, row.after, row.geometry)
                    ) : (
                      <span className="value-missing">—</span>
                    )}
                  </TableCell>
                )}
              </TableRow>
            ))}
          </TableBody>
        </Table>
        <GeometryPreview
          className="is-compact"
          layers={[
            {
              id: "before",
              label: beforeLabel,
              geometry: change.before?.geometry,
              present:
                !!change.before &&
                (change.kind === "deleted" || geometry.changed),
              tone: "before",
            },
            {
              id: "after",
              label: afterLabel,
              geometry: change.after?.geometry,
              present: !!change.after,
              tone: "after",
            },
          ]}
        />
      </div>
      <Button
        variant="ghost"
        size="xs"
        aria-expanded={raw}
        onClick={() => setRaw((v) => !v)}
      >
        <Braces />
        GeoJSON
      </Button>
      {raw && (
        <pre className="json-view">
          {stringify(change.after ?? change.before, undefined, 2)}
        </pre>
      )}
    </div>
  );
}

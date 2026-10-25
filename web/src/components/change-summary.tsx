"use client";
import { useEffect, useState } from "react";
import type { SchemaChange } from "@geoledger/client";
import { call } from "@/lib/browser-api";
import { count } from "@/lib/format";
import {
  sharedRequests,
  schemaFieldChanges,
  type ChangeSummary as Summary,
} from "@/lib/change-summary";
const loadSummary = sharedRequests<Summary>();
import { Button } from "./ui/button";
import { Skeleton } from "./ui/skeleton";
import {
  Table,
  TableCaption,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "./ui/table";

export function SchemaChangeSummary({
  changes,
}: {
  changes: readonly SchemaChange[];
}) {
  if (!changes.length) return null;
  return (
    <section className="space-y-2 text-xs" aria-label="字段变更">
      <strong>字段变更</strong>
      {changes.map((change) => {
        const fields = schemaFieldChanges(change);
        return (
          <div key={change.dataset}>
            <span>数据集 {change.dataset}</span>
            <ul className="space-y-1">
              {fields.added.length > 0 && (
                <li>新增字段：{fields.added.join("、")}</li>
              )}
              {fields.deleted.length > 0 && (
                <li>删除字段：{fields.deleted.join("、")}</li>
              )}
              {fields.modified.length > 0 && (
                <li>调整字段：{fields.modified.join("、")}</li>
              )}
            </ul>
          </div>
        );
      })}
    </section>
  );
}

export function ChangeSummary({
  project,
  workspace,
  revision,
  refresh,
  compact = false,
}: {
  project: string;
  workspace?: string;
  revision?: string;
  refresh?: string | number;
  compact?: boolean;
}) {
  const [state, setState] = useState<{
    key: string;
    value?: Summary;
    error?: string;
  }>();
  const [retry, setRetry] = useState(0);
  const key = JSON.stringify([project, workspace, revision, refresh, retry]);
  useEffect(() => {
    let active = true;
    loadSummary(key, () =>
      call<Summary>(
        workspace
          ? { action: "workspaceSummary", project, workspace }
          : { action: "commitSummary", project, revision },
      ),
    )
      .then((value) => {
        if (active) setState({ key, value });
      })
      .catch((error) => {
        if (active)
          setState({
            key,
            error: error instanceof Error ? error.message : "统计失败",
          });
      });
    return () => {
      active = false;
    };
  }, [key, project, workspace, revision]);
  const current = state?.key === key ? state : undefined;
  if (current?.error)
    return (
      <div className="text-xs text-destructive">
        统计失败：{current.error}{" "}
        <Button
          variant="ghost"
          size="xs"
          onClick={() => setRetry((n) => n + 1)}
        >
          重试统计
        </Button>
      </div>
    );
  if (!current?.value)
    return compact ? (
      <Skeleton className="h-4 w-36" aria-label="正在统计" />
    ) : (
      <Skeleton className="h-24 w-full" aria-label="正在统计" />
    );
  const { total, datasets, schemaChanges = [] } = current.value;
  if (compact)
    return (
      <div className="space-y-1 text-xs" aria-label="按数据集统计变更">
        {datasets.length ? (
          datasets.map((row) => (
            <div key={row.id} title={`数据集 ${row.name}（${row.id}）`}>
              <strong>{row.name}</strong>：新增 {row.added} · 删除 {row.deleted}{" "}
              · 修改 {row.modified}
            </div>
          ))
        ) : (
          <span className="muted">无要素变更</span>
        )}
        <SchemaChangeSummary changes={schemaChanges} />
        {datasets.length > 1 && (
          <div className="muted">
            合计：新增 {total.added} · 删除 {total.deleted} · 修改{" "}
            {total.modified}
          </div>
        )}
      </div>
    );
  return (
    <div className="space-y-3">
      <Table aria-label="按数据集统计要素变更" className="summary-table">
        <TableCaption className="sr-only">
          要素变更统计（全部记录）
        </TableCaption>
        <TableHeader>
          <TableRow>
            <TableHead>数据集</TableHead>
            <TableHead>新增要素</TableHead>
            <TableHead>删除要素</TableHead>
            <TableHead>修改要素</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {datasets.map((row) => (
            <TableRow key={row.id}>
              <TableCell title={row.id}>{row.name}</TableCell>
              <TableCell className="num is-added">{count(row.added)}</TableCell>
              <TableCell className="num is-deleted">
                {count(row.deleted)}
              </TableCell>
              <TableCell className="num is-modified">
                {count(row.modified)}
              </TableCell>
            </TableRow>
          ))}
          <TableRow className="total-row">
            <TableCell>合计</TableCell>
            <TableCell className="num">{count(total.added)}</TableCell>
            <TableCell className="num">{count(total.deleted)}</TableCell>
            <TableCell className="num">{count(total.modified)}</TableCell>
          </TableRow>
        </TableBody>
      </Table>
      <SchemaChangeSummary changes={schemaChanges} />
    </div>
  );
}

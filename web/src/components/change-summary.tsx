"use client";
import { useEffect, useState } from "react";
import { call } from "@/lib/browser-api";
import {
  sharedRequests,
  type ChangeSummary as Summary,
} from "@/lib/change-summary";
const loadSummary = sharedRequests<Summary>();
import { Button } from "./ui/button";
import {
  Table,
  TableCaption,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "./ui/table";

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
  if (!current?.value) return <span className="muted text-xs">统计中…</span>;
  const { total, datasets } = current.value;
  if (compact)
    return (
      <div className="space-y-1 text-xs" aria-label="按数据集统计要素变更">
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
        {datasets.length > 1 && (
          <div className="muted">
            合计：新增 {total.added} · 删除 {total.deleted} · 修改{" "}
            {total.modified}
          </div>
        )}
      </div>
    );
  return (
    <Table aria-label="按数据集统计要素变更">
      <TableCaption className="caption-top p-2 text-left font-medium">
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
            <TableCell>{row.added}</TableCell>
            <TableCell>{row.deleted}</TableCell>
            <TableCell>{row.modified}</TableCell>
          </TableRow>
        ))}
        <TableRow>
          <TableCell>合计</TableCell>
          <TableCell>{total.added}</TableCell>
          <TableCell>{total.deleted}</TableCell>
          <TableCell>{total.modified}</TableCell>
        </TableRow>
      </TableBody>
    </Table>
  );
}

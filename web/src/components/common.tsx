"use client";
import {
  ArrowLeft,
  ArrowRight,
  CircleAlert,
  Inbox,
  RefreshCw,
  type LucideIcon,
} from "lucide-react";
import { useEffect, useRef, useState, type ReactNode } from "react";
import { Button } from "./ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "./ui/dialog";
import { Skeleton } from "./ui/skeleton";
export function Empty({
  title = "这里还没有内容",
  icon: Icon = Inbox,
  children,
  action,
}: {
  title?: string;
  icon?: LucideIcon;
  children?: ReactNode;
  action?: ReactNode;
}) {
  return (
    <div className="empty">
      <span className="empty-icon">
        <Icon />
      </span>
      <h3>{title}</h3>
      {children && <p>{children}</p>}
      {action && <div className="empty-action">{action}</div>}
    </div>
  );
}
export function Loading({ rows = 5 }: { rows?: number }) {
  return (
    <div className="loading" role="status" aria-label="正在加载">
      {Array.from({ length: rows }, (_, i) => (
        <div className="loading-row" key={i}>
          <Skeleton className="loading-avatar" />
          <Skeleton className="loading-line" />
          <Skeleton className="loading-line is-short" />
        </div>
      ))}
    </div>
  );
}
export function ErrorBox({ message }: { message: string }) {
  return message ? (
    <div role="alert" className="alert">
      <CircleAlert aria-hidden="true" />
      <span>{message}</span>
    </div>
  ) : null;
}
export function Modal({
  title,
  description,
  children,
  close,
}: {
  title: string;
  description?: string;
  children: ReactNode;
  close: () => void;
}) {
  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) close();
      }}
    >
      <DialogContent
        className="dialog-wide"
        {...(description ? {} : { "aria-describedby": undefined })}
      >
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          {description && <DialogDescription>{description}</DialogDescription>}
        </DialogHeader>
        {children}
      </DialogContent>
    </Dialog>
  );
}
export function usePage<T>(
  load: (after: string) => Promise<{
    rows: T[];
    next?: string;
    identity?: string;
    accept?: () => void;
  }>,
  refresh: number,
) {
  const [cursors, setCursors] = useState([""]);
  const [rows, setRows] = useState<T[]>([]);
  const [next, setNext] = useState<string>();
  const [busy, setBusy] = useState(true);
  const [error, setError] = useState("");
  const [retry, setRetry] = useState(0);
  const identity = useRef<string | undefined>(undefined);
  const after = cursors.at(-1)!;
  useEffect(() => {
    let active = true;
    setBusy(true);
    setError("");
    setRows([]);
    setNext(undefined);
    load(after)
      .then((data) => {
        if (active) {
          if (
            after &&
            identity.current !== undefined &&
            data.identity !== identity.current
          )
            throw new Error("审阅内容已变化，请刷新列表，从第一页重新查看。");
          identity.current = data.identity;
          data.accept?.();
          setRows(data.rows);
          setNext(data.next);
        }
      })
      .catch((e) => {
        if (active) setError(e.message);
      })
      .finally(() => {
        if (active) setBusy(false);
      });
    return () => {
      active = false;
    };
    // The owning page remounts when project, dataset, workspace or operation changes.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [after, refresh, retry]);
  const reset = () => {
    setCursors([""]);
    setRetry((n) => n + 1);
  };
  const footer = (
    <div className="pagination">
      <span className="pagination-summary">
        第 {cursors.length} 页 · 本页 {rows.length} 项
      </span>
      <div className="pagination-actions">
        <Button
          variant="ghost"
          size="icon-sm"
          onClick={reset}
          disabled={busy}
          aria-label="刷新列表"
          title="刷新列表"
        >
          <RefreshCw className={busy ? "spin" : undefined} />
        </Button>
        <Button
          variant="outline"
          size="sm"
          disabled={busy || cursors.length === 1}
          onClick={() => setCursors((c) => c.slice(0, -1))}
        >
          <ArrowLeft />
          上一页
        </Button>
        <Button
          variant="outline"
          size="sm"
          disabled={busy || !next}
          onClick={() => setCursors((c) => [...c, next!])}
        >
          下一页
          <ArrowRight />
        </Button>
      </div>
    </div>
  );
  return { rows, busy, error, footer, reset };
}
export function listPage<T extends { id: string }>(rows: T[]) {
  return { rows, next: rows.length === 20 ? rows.at(-1)!.id : undefined };
}

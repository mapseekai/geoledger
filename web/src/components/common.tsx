"use client";
import {
  ArrowLeft,
  ArrowRight,
  CircleAlert,
  Inbox,
  RefreshCw,
  type LucideIcon,
} from "lucide-react";
import {
  useEffect,
  useRef,
  useState,
  type ComponentProps,
  type ReactElement,
  type ReactNode,
} from "react";
import { Alert, AlertDescription } from "./ui/alert";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "./ui/alert-dialog";
import { Button } from "./ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "./ui/dialog";
import { Skeleton } from "./ui/skeleton";
import { Tooltip, TooltipContent, TooltipTrigger } from "./ui/tooltip";
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
    <Alert variant="destructive" className="alert">
      <CircleAlert aria-hidden="true" />
      <AlertDescription>{message}</AlertDescription>
    </Alert>
  ) : null;
}
/** Non-error feedback; uses the Alert surface without the alert role. */
export function Notice({
  tone,
  icon,
  role,
  action,
  children,
}: {
  tone?: "success" | "warning" | "muted";
  icon?: ReactNode;
  role?: "status";
  action?: ReactNode;
  children: ReactNode;
}) {
  return (
    <Alert
      role={role ?? "note"}
      className={`notice${tone ? ` is-${tone}` : ""}`}
    >
      {icon}
      <AlertDescription>{children}</AlertDescription>
      {action}
    </Alert>
  );
}
/** Tooltip for icon buttons; the trigger keeps its own accessible name. */
export function Tip({
  label,
  side,
  container,
  children,
}: {
  label: ReactNode;
  side?: ComponentProps<typeof TooltipContent>["side"];
  container?: HTMLElement | null;
  children: ReactElement;
}) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>{children}</TooltipTrigger>
      <TooltipContent side={side} container={container ?? undefined}>
        {label}
      </TooltipContent>
    </Tooltip>
  );
}
/** Destructive confirmation; stays open while the action runs. */
export function Confirm({
  title,
  description,
  action,
  busy,
  error,
  confirm,
  close,
}: {
  title: string;
  description: string;
  action: string;
  busy: boolean;
  error: string;
  confirm: () => void;
  close: () => void;
}) {
  return (
    <AlertDialog
      open
      onOpenChange={(open) => {
        if (!open && !busy) close();
      }}
    >
      <AlertDialogContent className="dialog-wide">
        <AlertDialogHeader>
          <AlertDialogTitle>{title}</AlertDialogTitle>
          <AlertDialogDescription>{description}</AlertDialogDescription>
        </AlertDialogHeader>
        <ErrorBox message={error} />
        <AlertDialogFooter className="form-actions">
          <AlertDialogCancel disabled={busy}>取消</AlertDialogCancel>
          <AlertDialogAction
            variant="destructive"
            disabled={busy}
            onClick={(event) => {
              // Keep the dialog open until the request finishes or fails.
              event.preventDefault();
              confirm();
            }}
          >
            {action}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
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
/** Tracks a CSS media query; false during server rendering. */
export function useMedia(query: string) {
  const [matches, setMatches] = useState(false);
  useEffect(() => {
    const list = window.matchMedia(query);
    const update = () => setMatches(list.matches);
    update();
    list.addEventListener("change", update);
    return () => list.removeEventListener("change", update);
  }, [query]);
  return matches;
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
        <Tip label="刷新列表">
          <Button
            variant="ghost"
            size="icon-sm"
            onClick={reset}
            disabled={busy}
            aria-label="刷新列表"
          >
            <RefreshCw className={busy ? "spin" : undefined} />
          </Button>
        </Tip>
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

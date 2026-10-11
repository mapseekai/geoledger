"use client";
import { count, fullTime, relativeTime } from "@/lib/format";
import { cn } from "@/lib/utils";
import {
  ArrowLeft,
  ArrowRight,
  Check,
  CircleAlert,
  Copy,
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
import { toast } from "sonner";
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
import { Sheet, SheetContent, SheetDescription, SheetTitle } from "./ui/sheet";
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
export function ErrorBox({
  message,
  retry,
}: {
  message: string;
  retry?: () => void;
}) {
  return message ? (
    <Alert variant="destructive" className="alert">
      <CircleAlert aria-hidden="true" />
      <AlertDescription>{message}</AlertDescription>
      {retry && (
        <Button
          variant="outline"
          size="xs"
          className="alert-action"
          onClick={retry}
        >
          <RefreshCw />
          重试
        </Button>
      )}
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
  description?: ReactNode;
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
      <AlertDialogContent
        className="gl-dialog is-sm is-alert"
        aria-describedby={description ? undefined : undefined}
      >
        <AlertDialogHeader className="gl-dialog-header">
          <AlertDialogTitle>{title}</AlertDialogTitle>
        </AlertDialogHeader>
        <div className="gl-dialog-body">
          {description && (
            <AlertDialogDescription className="gl-dialog-text">
              {description}
            </AlertDialogDescription>
          )}
          <ErrorBox message={error} />
        </div>
        <AlertDialogFooter className="gl-dialog-footer">
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
            {busy && <RefreshCw className="spin" />}
            {action}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
function phoneQuery() {
  return (
    typeof window !== "undefined" &&
    window.matchMedia("(max-width: 640px)").matches
  );
}
export type ModalSize = "sm" | "md" | "lg" | "xl";
/**
 * One dialog frame for the whole console: sticky header and footer, a scrolling
 * body, fixed widths per size, and a bottom sheet on phones.
 */
export function Modal({
  title,
  meta,
  description,
  children,
  footer,
  close,
  busy = false,
  size = "md",
  className,
  bodyClassName,
}: {
  title: ReactNode;
  /** Small data shown beside the title (IDs, revisions, status). */
  meta?: ReactNode;
  /** Data line under the title. Not for instructions. */
  description?: ReactNode;
  children: ReactNode;
  footer?: ReactNode;
  close: () => void;
  busy?: boolean;
  size?: ModalSize;
  className?: string;
  bodyClassName?: string;
}) {
  // Dialogs only mount after user interaction, so reading the viewport is safe.
  const [phone, setPhone] = useState(phoneQuery);
  useEffect(() => {
    const list = window.matchMedia("(max-width: 640px)");
    const update = () => setPhone(list.matches);
    list.addEventListener("change", update);
    return () => list.removeEventListener("change", update);
  }, []);
  const onOpenChange = (open: boolean) => {
    if (!open && !busy) close();
  };
  const frame = cn("gl-dialog", `is-${size}`, className);
  // Start on the frame itself: no stray focus ring on the first footer button,
  // Esc works at once, and autoFocus fields still win (Radix skips this then).
  const focusFrame = (event: Event) => {
    event.preventDefault();
    (event.currentTarget as HTMLElement | null)?.focus();
  };
  const inner = (
    <>
      <div className="gl-dialog-body-wrap">
        <div className={cn("gl-dialog-body", bodyClassName)}>{children}</div>
      </div>
      {footer && <div className="gl-dialog-footer">{footer}</div>}
    </>
  );
  if (phone)
    return (
      <Sheet open onOpenChange={onOpenChange}>
        <SheetContent
          side="bottom"
          className={cn(frame, "is-sheet")}
          aria-describedby={undefined}
          onOpenAutoFocus={focusFrame}
        >
          <div className="gl-dialog-header">
            <SheetTitle>{title}</SheetTitle>
            {meta && <div className="gl-dialog-meta">{meta}</div>}
            {description && (
              <SheetDescription className="gl-dialog-subtitle">
                {description}
              </SheetDescription>
            )}
          </div>
          {inner}
        </SheetContent>
      </Sheet>
    );
  return (
    <Dialog open onOpenChange={onOpenChange}>
      <DialogContent
        className={frame}
        aria-describedby={undefined}
        onOpenAutoFocus={focusFrame}
      >
        <DialogHeader className="gl-dialog-header">
          <DialogTitle>{title}</DialogTitle>
          {meta && <div className="gl-dialog-meta">{meta}</div>}
          {description && (
            <DialogDescription className="gl-dialog-subtitle">
              {description}
            </DialogDescription>
          )}
        </DialogHeader>
        {inner}
      </DialogContent>
    </Dialog>
  );
}
/** Footer group pinned to the left edge (status, progress). */
export function FooterStart({ children }: { children: ReactNode }) {
  return <div className="gl-dialog-footer-start">{children}</div>;
}
export function copyText(value: string, label = "已复制") {
  void navigator.clipboard
    ?.writeText(value)
    .then(() => toast.success(label, { description: value }))
    .catch(() => toast.error("复制失败"));
}
/** Icon button that copies an identifier or revision. */
export function CopyButton({
  value,
  label = "复制",
  className,
}: {
  value: string;
  label?: string;
  className?: string;
}) {
  const [done, setDone] = useState(false);
  useEffect(() => {
    if (!done) return;
    const timer = setTimeout(() => setDone(false), 1400);
    return () => clearTimeout(timer);
  }, [done]);
  return (
    <Tip label={done ? "已复制" : label}>
      <Button
        type="button"
        variant="ghost"
        size="icon-xs"
        className={cn("copy-button", className)}
        aria-label={`${label} ${value}`}
        onClick={(event) => {
          event.stopPropagation();
          copyText(value);
          setDone(true);
        }}
      >
        {done ? <Check /> : <Copy />}
      </Button>
    </Tip>
  );
}
/** Monospace identifier with a full-value tooltip and copy button. */
export function IdText({
  value,
  short,
  copy = true,
  className,
}: {
  value: string;
  short?: string;
  copy?: boolean;
  className?: string;
}) {
  return (
    <span className={cn("id-text", className)}>
      <span className="mono" title={value}>
        {short ?? value}
      </span>
      {copy && <CopyButton value={value} label="复制标识" />}
    </span>
  );
}
/** Relative time with the exact local time in a tooltip. */
export function RelativeTime({
  value,
  plain = false,
}: {
  value: string;
  /** Inside another control: native title instead of a focusable tooltip. */
  plain?: boolean;
}) {
  const [now, setNow] = useState<number>();
  useEffect(() => {
    setNow(Date.now());
    const timer = setInterval(() => setNow(Date.now()), 60_000);
    return () => clearInterval(timer);
  }, []);
  const text = now === undefined ? fullTime(value) : relativeTime(value, now);
  if (plain)
    return (
      <time dateTime={value} className="relative-time" title={fullTime(value)}>
        {text}
      </time>
    );
  return (
    <Tip label={fullTime(value)}>
      <time dateTime={value} className="relative-time" tabIndex={0}>
        {text}
      </time>
    </Tip>
  );
}
/** "+3 ~2 −1" chips; zero parts stay visible but muted. */
export function DiffStat({
  added,
  modified,
  deleted,
  className,
}: {
  added?: string | number | bigint;
  modified?: string | number | bigint;
  deleted?: string | number | bigint;
  className?: string;
}) {
  const part = (kind: string, sign: string, label: string, value: unknown) => (
    <span
      className={cn("diff-stat-part", `is-${kind}`, {
        "is-zero": String(value ?? "0") === "0",
      })}
      title={`${label} ${count(value as string)}`}
    >
      <span aria-hidden="true">{sign}</span>
      <span className="sr-only">{label} </span>
      {count(value as string)}
    </span>
  );
  return (
    <span className={cn("diff-stat", className)}>
      {part("added", "+", "新增", added)}
      {part("modified", "~", "修改", modified)}
      {part("deleted", "−", "删除", deleted)}
    </span>
  );
}
/** Skeleton rows sized like list items, for dialogs and panels. */
export function ListSkeleton({ rows = 4 }: { rows?: number }) {
  return (
    <div className="list-skeleton" role="status" aria-label="正在加载">
      {Array.from({ length: rows }, (_, i) => (
        <div key={i}>
          <Skeleton className="h-4 w-14" />
          <Skeleton className="h-4 flex-1" />
          <Skeleton className="h-4 w-20" />
        </div>
      ))}
    </div>
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
  refresh: string | number,
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
  return {
    rows,
    busy,
    error,
    footer,
    reset,
    hasPrevious: cursors.length > 1,
  };
}
export function listPage<T extends { id: string }>(rows: T[]) {
  return { rows, next: rows.length === 20 ? rows.at(-1)!.id : undefined };
}

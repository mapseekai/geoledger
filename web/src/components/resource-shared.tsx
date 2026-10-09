"use client";
import { useState, type ReactNode } from "react";
import { ErrorBox, Modal } from "./common";
import { Badge } from "./ui/badge";
import { Button } from "./ui/button";
import { Input } from "./ui/input";
import { Label } from "./ui/label";
const labels: Record<string, string> = {
  owner: "所有者",
  editor: "编辑者",
  viewer: "只读",
  open: "编辑中",
  published: "已发布",
  discarded: "已丢弃",
  unpublished: "未发布",
};
export const role = (v: string) => labels[v] ?? v;
export const short = (v: string) =>
  v.length > 20 ? `${v.slice(0, 8)}…${v.slice(-6)}` : v;
export const time = (v: string) =>
  new Date(v).toLocaleString("zh-CN", { hour12: false });
export function useAction() {
  const [busy, setBusy] = useState(false),
    [error, setError] = useState("");
  async function run(fn: () => Promise<void>) {
    if (busy) return;
    setBusy(true);
    setError("");
    try {
      await fn();
    } catch (e) {
      setError(e instanceof Error ? e.message : "操作失败");
    } finally {
      setBusy(false);
    }
  }
  return { busy, error, run };
}
export function Panel({
  children,
  toolbar,
  className,
}: {
  children: ReactNode;
  toolbar?: ReactNode;
  className?: string;
}) {
  return (
    <section className={className ? `panel ${className}` : "panel"}>
      {toolbar && <div className="panel-toolbar">{toolbar}</div>}
      {children}
    </section>
  );
}
/** Card title block used at the start of a panel toolbar. */
export function PanelTitle({
  title,
  description,
  children,
}: {
  title: string;
  description?: string;
  children?: ReactNode;
}) {
  return (
    <div className="panel-title">
      <div className="panel-title-row">
        <strong>{title}</strong>
        {children}
      </div>
      {description && <span>{description}</span>}
    </div>
  );
}
const tones: Record<string, string> = {
  owner: "tone-brand",
  editor: "tone-info",
  viewer: "tone-neutral",
  open: "tone-info",
  published: "tone-success",
  discarded: "tone-neutral",
  unpublished: "tone-neutral",
};
/** Colored pill for roles and workspace states; text comes from `role()`. */
export function StatusBadge({ value }: { value: string }) {
  return (
    <Badge
      variant="outline"
      className={`status-badge ${tones[value] ?? "tone-neutral"}`}
    >
      <span className="status-badge-dot" aria-hidden="true" />
      {role(value)}
    </Badge>
  );
}
export function NameDialog({
  title,
  close,
  submit,
  initialName = "",
  children,
  actionLabel = "创建",
  description,
  confirmName,
}: {
  title: string;
  initialName?: string;
  children?: ReactNode;
  actionLabel?: string;
  description?: string;
  confirmName?: string;
  close: () => void;
  submit: (name: string) => Promise<void>;
}) {
  const task = useAction();
  return (
    <Modal
      title={title}
      description={description}
      close={task.busy ? () => {} : close}
    >
      <form
        onSubmit={(e) => {
          e.preventDefault();
          const entered = String(new FormData(e.currentTarget).get("name"));
          const name = confirmName === undefined ? entered.trim() : entered;
          void task.run(async () => {
            if (confirmName !== undefined && name !== confirmName)
              throw new Error("请输入完全一致的名称以确认删除。");
            await submit(name);
          });
        }}
      >
        <Label htmlFor="name">
          {confirmName === undefined ? "名称" : "输入名称确认删除"}
        </Label>
        <Input
          id="name"
          name="name"
          defaultValue={initialName}
          autoFocus
          required
          maxLength={256}
        />
        {children}
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
            {task.busy ? "正在处理…" : actionLabel}
          </Button>
        </div>
      </form>
    </Modal>
  );
}

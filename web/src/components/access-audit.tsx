"use client";
import { PasswordInput } from "./password-input";
import { validatePasswordForm } from "@/lib/password";
import { call, type Audit, type Member, type Project } from "@/lib/browser-api";
import { pretty } from "@/lib/geojson";
import { ScrollText, Users } from "lucide-react";
import { useState, useRef, type ReactNode } from "react";
import { toast } from "sonner";
import {
  copyText,
  Empty,
  ErrorBox,
  Loading,
  Modal,
  RelativeTime,
  usePage,
} from "./common";
import { Panel, PanelTitle, useAction } from "./resource-shared";
import { Button } from "./ui/button";
import { Badge } from "./ui/badge";
import { Input } from "./ui/input";
import { Field, FieldGroup, FieldLabel } from "./ui/field";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectGroup,
  SelectTrigger,
  SelectValue,
} from "./ui/select";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "./ui/table";
export function Access({ project }: { project: Project }) {
  const [editing, setEditing] = useState(false);
  const trigger = useRef<HTMLButtonElement>(null);
  const [refresh, setRefresh] = useState(0);
  const owner = project.role === "owner" || project.role === "admin";
  return (
    <>
      <Members
        project={project}
        refresh={refresh}
        actions={
          <Button
            ref={trigger}
            disabled={!owner}
            onClick={() => setEditing(true)}
          >
            设置成员权限
          </Button>
        }
      />
      {editing && (
        <MemberDialog
          project={project}
          close={() => setEditing(false)}
          restoreFocus={() => trigger.current?.focus()}
          saved={() => {
            setRefresh((n) => n + 1);
            setEditing(false);
            toast.success("成员权限已更新。");
          }}
        />
      )}
    </>
  );
}
function MemberDialog({
  project,
  close,
  saved,
  restoreFocus,
}: {
  project: Project;
  close: () => void;
  saved: () => void;
  restoreFocus: () => void;
}) {
  const task = useAction();
  return (
    <Modal
      title="设置成员权限"
      onCloseAutoFocus={(event) => {
        event.preventDefault();
        restoreFocus();
      }}
      size="sm"
      close={close}
      busy={task.busy}
      footer={
        <>
          <Button
            type="button"
            variant="outline"
            disabled={task.busy}
            onClick={close}
          >
            取消
          </Button>
          <Button type="submit" form="member-permissions" disabled={task.busy}>
            {task.busy ? "正在保存…" : "保存权限"}
          </Button>
        </>
      }
    >
      <form
        id="member-permissions"
        onSubmit={(event) => {
          event.preventDefault();
          if (!validatePasswordForm(event.currentTarget)) return;
          const data = new FormData(event.currentTarget);
          void task.run(async () => {
            await call({
              action: "setMember",
              project: project.id,
              subject: String(data.get("subject")).trim(),
              role: String(data.get("role")),
              currentPassword: String(data.get("currentPassword")),
            });
            saved();
          });
        }}
      >
        <FieldGroup>
          <Field>
            <FieldLabel htmlFor="subject">成员身份</FieldLabel>
            <Input
              id="subject"
              name="subject"
              required
              autoFocus
              placeholder="用户管理页面中的固定用户 ID"
              maxLength={128}
              disabled={task.busy}
            />
          </Field>
          <Field>
            <FieldLabel htmlFor="member-role">项目角色</FieldLabel>
            <Select name="role" defaultValue="viewer" disabled={task.busy}>
              <SelectTrigger id="member-role">
                <SelectValue />
              </SelectTrigger>
              <SelectContent position="popper" className="menu">
                <SelectGroup>
                  <SelectItem value="viewer">只读 · 浏览与查询</SelectItem>
                  <SelectItem value="editor">编辑者 · 编辑与发布</SelectItem>
                  <SelectItem value="owner">所有者 · 管理项目与权限</SelectItem>
                </SelectGroup>
              </SelectContent>
            </Select>
          </Field>
          <Field>
            <FieldLabel htmlFor="member-password">当前密码</FieldLabel>
            <PasswordInput
              id="member-password"
              name="currentPassword"
              type="password"
              autoComplete="current-password"
              required
              disabled={task.busy}
            />
            <small>UTF-8 1–1024 字节</small>
          </Field>
          <ErrorBox message={task.error} />
        </FieldGroup>
      </form>
    </Modal>
  );
}
const roleLabels: Record<string, string> = {
  owner: "所有者",
  editor: "编辑者",
  viewer: "只读",
};
function Members({
  project,
  refresh,
  actions,
}: {
  project: Project;
  refresh: number;
  actions: ReactNode;
}) {
  const task = useAction();
  const [removed, setRemoved] = useState(0);
  const [removing, setRemoving] = useState<Member>();
  const owner = project.role === "owner" || project.role === "admin";
  const page = usePage<Member>(async (after) => {
    const rows = await call<Member[]>({
      action: "members",
      project: project.id,
      after: after || undefined,
      limit: 20,
    });
    return {
      rows,
      next: rows.length === 20 ? rows[rows.length - 1].subject : undefined,
    };
  }, refresh + removed);
  return (
    <Panel
      toolbar={
        <>
          <PanelTitle title="项目成员" />
          {actions}
        </>
      }
    >
      <ErrorBox message={page.error || task.error} />
      {page.busy ? (
        <Loading />
      ) : page.rows.length ? (
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>成员</TableHead>
              <TableHead>角色</TableHead>
              <TableHead className="cell-actions">操作</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {page.rows.map((m) => (
              <TableRow key={m.subject}>
                <TableCell>
                  <span className="subject">{m.subject}</span>
                </TableCell>
                <TableCell>{roleLabels[m.role] ?? m.role}</TableCell>
                <TableCell className="cell-actions">
                  <Button
                    variant="ghost"
                    size="sm"
                    disabled={!owner || task.busy}
                    aria-label={`移除成员 ${m.subject}`}
                    onClick={() => setRemoving(m)}
                  >
                    移除
                  </Button>
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      ) : (
        <Empty icon={Users} title="没有成员" />
      )}
      {page.footer}
      {removing && (
        <Modal
          title="移除成员"
          busy={task.busy}
          close={() => setRemoving(undefined)}
        >
          <form
            onSubmit={(event) => {
              event.preventDefault();
              if (!validatePasswordForm(event.currentTarget)) return;
              const currentPassword = String(
                new FormData(event.currentTarget).get("currentPassword"),
              );
              void task.run(async () => {
                await call({
                  action: "removeMember",
                  project: project.id,
                  subject: removing.subject,
                  currentPassword,
                });
                setRemoved((n) => n + 1);
                setRemoving(undefined);
              });
            }}
          >
            <p>确认移除成员 {removing.subject} 的项目权限。</p>
            <Field>
              <FieldLabel htmlFor="remove-password">当前密码</FieldLabel>
              <PasswordInput
                id="remove-password"
                name="currentPassword"
                type="password"
                autoComplete="current-password"
                required
              />
              <small>UTF-8 1–1024 字节</small>
            </Field>
            <ErrorBox message={task.error} />
            <Button type="submit" disabled={task.busy}>
              确认移除
            </Button>
          </form>
        </Modal>
      )}
    </Panel>
  );
}
export function AuditPanel({ project }: { project: Project }) {
  const [detail, setDetail] = useState<Audit>();
  const page = usePage<Audit>(async (after) => {
    const r = await call<{ events: Audit[]; nextAfter?: string }>({
      action: "audit",
      project: project.id,
      after: after || "0",
      limit: 20,
    });
    return { rows: r.events, next: r.nextAfter };
  }, 0);
  return (
    <>
      <Panel toolbar={<PanelTitle title="项目操作记录" />}>
        <ErrorBox message={page.error} />
        {page.busy ? (
          <Loading />
        ) : page.rows.length ? (
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>事件</TableHead>
                <TableHead>操作</TableHead>
                <TableHead>成员</TableHead>
                <TableHead>时间</TableHead>
                <TableHead className="cell-actions">详情</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {page.rows.map((e) => (
                <TableRow key={e.id}>
                  <TableCell className="mono muted">#{e.id}</TableCell>
                  <TableCell>
                    <Badge variant="secondary" className="code-tag">
                      {e.action}
                    </Badge>
                  </TableCell>
                  <TableCell>
                    <span className="subject">
                      <span className="subject-avatar" aria-hidden="true">
                        {e.subject.slice(0, 1).toUpperCase()}
                      </span>
                      {e.subject}
                    </span>
                  </TableCell>
                  <TableCell className="muted">
                    <RelativeTime value={e.createdAt} />
                  </TableCell>
                  <TableCell className="cell-actions">
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={() => setDetail(e)}
                    >
                      查看
                    </Button>
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        ) : (
          <Empty icon={ScrollText} title="还没有审计记录" />
        )}
        {page.footer}
      </Panel>
      {detail && (
        <Modal
          title={`审计事件 #${detail.id}`}
          meta={
            <>
              <Badge variant="outline" className="meta-chip">
                {detail.subject}
              </Badge>
              <Badge variant="outline" className="meta-chip">
                <span className="mono">{detail.action}</span>
              </Badge>
              <Badge variant="outline" className="meta-chip">
                <RelativeTime value={detail.createdAt} plain />
              </Badge>
            </>
          }
          close={() => setDetail(undefined)}
          footer={
            <>
              <Button
                variant="outline"
                onClick={() => copyText(detail.detail, "已复制详情")}
              >
                复制
              </Button>
              <Button onClick={() => setDetail(undefined)}>关闭</Button>
            </>
          }
        >
          <pre className="json-view">{pretty(detail.detail)}</pre>
        </Modal>
      )}
    </>
  );
}

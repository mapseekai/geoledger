"use client";
import { call, type Audit, type Member, type Project } from "@/lib/browser-api";
import { pretty } from "@/lib/geojson";
import { CircleCheck, Lock, ScrollText, Users } from "lucide-react";
import { useState } from "react";
import {
  copyText,
  Empty,
  ErrorBox,
  Loading,
  Modal,
  Notice,
  RelativeTime,
  usePage,
} from "./common";
import { Panel, PanelTitle, useAction } from "./resource-shared";
import { Button } from "./ui/button";
import { Badge } from "./ui/badge";
import { Input } from "./ui/input";
import { Label } from "./ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
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
  const task = useAction();
  const [success, setSuccess] = useState("");
  const [refresh, setRefresh] = useState(0);
  const owner = project.role === "owner";
  return (
    <div className="form-grid">
      <Panel toolbar={<PanelTitle title="设置成员权限" />}>
        <form
          className="card-form"
          onSubmit={(e) => {
            e.preventDefault();
            const data = new FormData(e.currentTarget);
            setSuccess("");
            void task.run(async () => {
              await call({
                action: "setMember",
                project: project.id,
                subject: String(data.get("subject")).trim(),
                role: String(data.get("role")),
              });
              setSuccess("成员权限已更新。");
              setRefresh((n) => n + 1);
            });
          }}
        >
          {!owner && (
            <Notice tone="muted" icon={<Lock aria-hidden="true" />}>
              只有项目所有者可以管理成员权限。
            </Notice>
          )}
          <div className="field">
            <Label htmlFor="subject">成员身份</Label>
            <Input
              id="subject"
              name="subject"
              required
              placeholder="令牌或身份提供方中的 subject"
              maxLength={128}
            />
          </div>
          <div className="field">
            <Label htmlFor="member-role">项目角色</Label>
            <Select name="role" defaultValue="viewer">
              <SelectTrigger id="member-role" className="role-select">
                <SelectValue />
              </SelectTrigger>
              <SelectContent position="popper" className="menu">
                <SelectItem value="viewer">只读 · 浏览与查询</SelectItem>
                <SelectItem value="editor">编辑者 · 编辑与发布</SelectItem>
                <SelectItem value="owner">所有者 · 管理项目与权限</SelectItem>
              </SelectContent>
            </Select>
          </div>
          <ErrorBox message={task.error} />
          {success && (
            <Notice
              role="status"
              tone="success"
              icon={<CircleCheck aria-hidden="true" />}
            >
              {success}
            </Notice>
          )}
          <div className="form-actions is-start">
            <Button disabled={!owner || task.busy}>
              {task.busy ? "正在保存…" : "保存权限"}
            </Button>
          </div>
        </form>
      </Panel>
      <Members project={project} refresh={refresh} />
      {owner && <Lifecycle project={project} />}
    </div>
  );
}
const roleLabels: Record<string, string> = {
  owner: "所有者",
  editor: "编辑者",
  viewer: "只读",
};
function Members({ project, refresh }: { project: Project; refresh: number }) {
  const task = useAction();
  const [removed, setRemoved] = useState(0);
  const owner = project.role === "owner";
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
    <Panel toolbar={<PanelTitle title="项目成员" />}>
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
                    onClick={() =>
                      void task.run(async () => {
                        await call({
                          action: "removeMember",
                          project: project.id,
                          subject: m.subject,
                        });
                        setRemoved((n) => n + 1);
                      })
                    }
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
    </Panel>
  );
}
function Lifecycle({ project }: { project: Project }) {
  const task = useAction();
  const [state, setState] = useState(project.state);
  const archived = state === "archived";
  return (
    <Panel toolbar={<PanelTitle title="项目状态" />}>
      <div className="card-form">
        <p className="muted">{archived ? "已归档 · 只读" : "活跃"}</p>
        <div className="form-actions is-start">
          <Button
            variant="outline"
            disabled={task.busy}
            onClick={() =>
              void task.run(async () => {
                const next = await call<Project>({
                  action: "archiveProject",
                  project: project.id,
                  archived: !archived,
                });
                setState(next.state);
              })
            }
          >
            {archived ? "恢复项目" : "归档项目"}
          </Button>
        </div>
        <form
          className="card-form"
          onSubmit={(e) => {
            e.preventDefault();
            const confirmName = String(
              new FormData(e.currentTarget).get("confirm"),
            );
            void task.run(async () => {
              await call({
                action: "deleteProject",
                project: project.id,
                confirmName,
              });
              window.location.assign("/projects");
            });
          }}
        >
          <div className="field">
            <Label htmlFor="confirm-delete">
              删除项目：输入项目名称“{project.name}”确认
            </Label>
            <Input
              id="confirm-delete"
              name="confirm"
              required
              maxLength={256}
            />
          </div>
          <ErrorBox message={task.error} />
          <div className="form-actions is-start">
            <Button variant="destructive" disabled={task.busy}>
              删除项目
            </Button>
          </div>
        </form>
      </div>
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

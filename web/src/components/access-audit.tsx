"use client";
import { call, type Audit, type Project } from "@/lib/browser-api";
import { pretty } from "@/lib/geojson";
import { CircleCheck, Lock, ScrollText } from "lucide-react";
import { useState } from "react";
import { Empty, ErrorBox, Loading, Modal, usePage } from "./common";
import { Panel, PanelTitle, time, useAction } from "./resource-shared";
import { Button } from "./ui/button";
import { Input } from "./ui/input";
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
            });
          }}
        >
          {!owner && (
            <div className="notice is-muted">
              <Lock aria-hidden="true" />
              <span>只有项目所有者可以管理成员权限。</span>
            </div>
          )}
          <div className="field">
            <label htmlFor="subject">成员身份</label>
            <Input
              id="subject"
              name="subject"
              required
              placeholder="令牌或身份提供方中的 subject"
              maxLength={128}
            />
          </div>
          <div className="field">
            <label htmlFor="member-role">项目角色</label>
            <select id="member-role" name="role">
              <option value="viewer">只读 · 浏览与查询</option>
              <option value="editor">编辑者 · 编辑与发布</option>
              <option value="owner">所有者 · 管理项目与权限</option>
            </select>
          </div>
          <ErrorBox message={task.error} />
          {success && (
            <div role="status" className="notice is-success">
              <CircleCheck aria-hidden="true" />
              <span>{success}</span>
            </div>
          )}
          <div className="form-actions is-start">
            <Button disabled={!owner || task.busy}>
              {task.busy ? "正在保存…" : "保存权限"}
            </Button>
          </div>
        </form>
      </Panel>
    </div>
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
                    <span className="code-tag">{e.action}</span>
                  </TableCell>
                  <TableCell>
                    <span className="subject">
                      <span className="subject-avatar" aria-hidden="true">
                        {e.subject.slice(0, 1).toUpperCase()}
                      </span>
                      {e.subject}
                    </span>
                  </TableCell>
                  <TableCell className="muted">{time(e.createdAt)}</TableCell>
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
          description={`${detail.subject} · ${detail.action}`}
          close={() => setDetail(undefined)}
        >
          <pre className="json-view">{pretty(detail.detail)}</pre>
        </Modal>
      )}
    </>
  );
}

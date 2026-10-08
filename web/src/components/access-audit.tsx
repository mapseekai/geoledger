"use client";
import { call, type Audit, type Project } from "@/lib/browser-api";
import { pretty } from "@/lib/geojson";
import { Shield } from "lucide-react";
import { useState } from "react";
import { Empty, ErrorBox, Loading, Modal, usePage } from "./common";
import { Panel, time, useAction } from "./resource-shared";
import { Badge } from "./ui/badge";
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
  return (
    <div className="service-grid">
      <Panel>
        <div className="card-heading">
          <Shield />
          <h2>设置成员权限</h2>
        </div>
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
          <label htmlFor="subject">成员身份</label>
          <Input
            id="subject"
            name="subject"
            required
            placeholder="令牌或身份提供方中的 subject"
            maxLength={128}
          />
          <label htmlFor="member-role">项目角色</label>
          <select id="member-role" name="role">
            <option value="viewer">只读 · 浏览与查询</option>
            <option value="editor">编辑者 · 编辑与发布</option>
            <option value="owner">所有者 · 管理项目与权限</option>
          </select>
          <ErrorBox message={task.error} />
          {success && (
            <div role="status" className="notice">
              {success}
            </div>
          )}
          <Button disabled={project.role !== "owner" || task.busy}>
            保存权限
          </Button>
          {project.role !== "owner" && (
            <p className="field-help">只有项目所有者可以管理成员权限。</p>
          )}
        </form>
      </Panel>
      <div className="cream-card">
        <span className="eyebrow">THE RIGHT ACCESS</span>
        <h2>
          让每个人，
          <br />
          各得其所。
        </h2>
        <p>
          权限绑定成员身份，而非浏览器或设备。请先由服务管理员为成员配置身份或访问令牌，再在这里授予项目权限。
        </p>
        <p>此处按身份设置权限。权限变更记录可在审计日志中查询。</p>
      </div>
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
      <Panel toolbar={<strong>项目操作记录</strong>}>
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
                <TableHead>详情</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {page.rows.map((e) => (
                <TableRow key={e.id}>
                  <TableCell className="mono">#{e.id}</TableCell>
                  <TableCell>
                    <Badge variant="secondary">{e.action}</Badge>
                  </TableCell>
                  <TableCell>{e.subject}</TableCell>
                  <TableCell className="muted">{time(e.createdAt)}</TableCell>
                  <TableCell>
                    <Button variant="ghost" onClick={() => setDetail(e)}>
                      查看
                    </Button>
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        ) : (
          <Empty title="还没有审计记录" />
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

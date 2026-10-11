"use client";
import { useEffect, useState } from "react";
import type { ManagedToken, IssuedToken } from "@geoledger/client";
import { call, type Member, type Project } from "@/lib/browser-api";
import { Empty, ErrorBox, Loading, Modal, Notice, usePage } from "./common";
import { Panel, PanelTitle, time, useAction } from "./resource-shared";
import { Button } from "./ui/button";
import { Checkbox } from "./ui/checkbox";
import { Calendar } from "./ui/calendar";
import { Popover, PopoverContent, PopoverTrigger } from "./ui/popover";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "./ui/select";
import { zhCN } from "react-day-picker/locale";
import { Input } from "./ui/input";
import { Label } from "./ui/label";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "./ui/table";
import { KeyRound } from "lucide-react";

type Pending = {
  action: "issueToken" | "reissueToken";
  requestId: string;
  subject?: string;
  tokenId?: string;
  label: string;
  expiresAt: string;
};
export function TokensPanel({ project }: { project?: Project }) {
  const [refresh, setRefresh] = useState(0);
  const [issuing, setIssuing] = useState<{ previous?: ManagedToken }>();
  const [revoking, setRevoking] = useState<ManagedToken>();
  const task = useAction();
  const page = usePage<ManagedToken>(async (after) => {
    const rows = await call<ManagedToken[]>({
      action: "tokens",
      after: after || undefined,
      limit: 20,
    });
    return {
      rows,
      next: rows.length === 20 ? rows[rows.length - 1].id : undefined,
    };
  }, refresh);
  return (
    <div className="access-layout">
      <Notice tone="muted">
        令牌用于登录，项目角色决定可访问的数据。此处仅管理控制台签发的个人令牌；运维文件凭据由部署管理员管理。请先在项目“访问权限”添加成员，再从当前项目成员中选择签发对象。
      </Notice>
      <Notice tone="muted">
        {project
          ? `签发范围：${project.name} 的现有成员。令牌按身份登录，可访问该身份获授权的所有项目。`
          : "请先选择项目，再为现有成员签发令牌。"}
      </Notice>
      <Panel
        toolbar={
          <>
            <PanelTitle
              title="个人访问令牌"
              description="明文仅展示一次，丢失后撤销并重新签发。"
            />
            <Button disabled={!project} onClick={() => setIssuing({})}>
              签发令牌
            </Button>
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
                <TableHead>成员 / 用途</TableHead>
                <TableHead>签发信息</TableHead>
                <TableHead>到期 / 状态</TableHead>
                <TableHead className="cell-actions">操作</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {page.rows.map((r) => (
                <TableRow key={r.id}>
                  <TableCell>
                    <strong>{r.subject}</strong>
                    <p className="muted">{r.label || "未填写用途"}</p>
                    <span className="mono text-xs">{r.id}</span>
                  </TableCell>
                  <TableCell>
                    {r.createdBy}
                    <p className="muted">{time(r.createdAt)}</p>
                  </TableCell>
                  <TableCell>
                    {time(r.expiresAt)}
                    <p className="muted">
                      {r.revokedAt
                        ? "已撤销"
                        : Date.parse(r.expiresAt) <= Date.now()
                          ? "已过期"
                          : "有效"}
                    </p>
                  </TableCell>
                  <TableCell className="cell-actions">
                    {!r.revokedAt && (
                      <div className="flex flex-wrap justify-end gap-2">
                        <Button
                          variant="outline"
                          size="sm"
                          onClick={() => setIssuing({ previous: r })}
                        >
                          重新签发
                        </Button>
                        <Button
                          variant="destructive"
                          size="sm"
                          onClick={() => setRevoking(r)}
                        >
                          撤销
                        </Button>
                      </div>
                    )}
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        ) : (
          <Empty icon={KeyRound} title="尚未签发个人令牌" />
        )}
        {page.footer}
      </Panel>
      {issuing && (
        <IssueDialog
          project={project}
          previous={issuing.previous}
          close={() => setIssuing(undefined)}
          changed={() => setRefresh((n) => n + 1)}
        />
      )}
      {revoking && (
        <Modal
          title="撤销令牌"
          description={`撤销 ${revoking.subject} 的“${revoking.label || "未填写用途"}”令牌后，使用它的后续请求将被拒绝。`}
          close={() => {
            if (!task.busy) setRevoking(undefined);
          }}
        >
          <ErrorBox message={task.error} />
          <div className="form-actions">
            <Button
              variant="outline"
              disabled={task.busy}
              onClick={() => setRevoking(undefined)}
            >
              取消
            </Button>
            <Button
              variant="destructive"
              disabled={task.busy}
              onClick={() =>
                void task.run(async () => {
                  await call({ action: "revokeToken", tokenId: revoking.id });
                  setRevoking(undefined);
                  setRefresh((n) => n + 1);
                })
              }
            >
              {task.busy ? "正在撤销…" : "确认撤销"}
            </Button>
          </div>
        </Modal>
      )}
    </div>
  );
}
function IssueDialog({
  project,
  previous,
  close,
  changed,
}: {
  project?: Project;
  previous?: ManagedToken;
  close: () => void;
  changed: () => void;
}) {
  const [expiry, setExpiry] = useState(
    () => new Date(Date.now() + 90 * 86400000),
  );
  const [members, setMembers] = useState<Member[]>([]);
  const [membersBusy, setMembersBusy] = useState(!previous);
  const [membersError, setMembersError] = useState("");
  const [retryMembers, setRetryMembers] = useState(0);
  const [subject, setSubject] = useState("");
  useEffect(() => {
    if (previous || !project) return;
    let active = true;
    setMembersBusy(true);
    setMembersError("");
    void (async () => {
      const all: Member[] = [];
      let after: string | undefined;
      do {
        const rows = await call<Member[]>({
          action: "members",
          project: project.id,
          after,
          limit: 100,
        });
        if (!active) return;
        all.push(...rows);
        if (rows.length < 100) break;
        after = rows[rows.length - 1].subject;
      } while (active);
      if (active) setMembers(all);
    })()
      .catch((e) => {
        if (active) setMembersError(e.message);
      })
      .finally(() => {
        if (active) setMembersBusy(false);
      });
    return () => {
      active = false;
    };
  }, [project?.id, previous, retryMembers]);
  const [calendarOpen, setCalendarOpen] = useState(false);
  const [pending, setPending] = useState<Pending>();
  const [result, setResult] = useState<IssuedToken>();
  const [saved, setSaved] = useState(false);
  const [copied, setCopied] = useState(false);
  const [copyError, setCopyError] = useState("");
  const task = useAction();
  useEffect(() => {
    if (!result?.token && !pending) return;
    const guard = (e: BeforeUnloadEvent) => {
      e.preventDefault();
      e.returnValue = "";
    };
    window.addEventListener("beforeunload", guard);
    return () => window.removeEventListener("beforeunload", guard);
  }, [result, pending]);
  const closeDialog = () => {
    if (task.busy || (result?.token && !saved)) return;
    setResult(undefined);
    setPending(undefined);
    close();
  };
  const submit = (request: Pending) =>
    void task.run(async () => {
      const response = await call<IssuedToken>(request);
      setResult(response);
      setPending(undefined);
      changed();
    });
  return (
    <Modal
      title={result ? "签发结果" : previous ? "重新签发令牌" : "签发个人令牌"}
      description={
        result?.token
          ? "请立即复制并安全保存，关闭后无法再次查看。"
          : previous
            ? `旧令牌将在新令牌签发成功时撤销，成员身份仍为 ${previous.subject}。`
            : "令牌不会自动授予项目权限。"
      }
      close={closeDialog}
    >
      {result ? (
        <div className="token-result grid min-w-0 gap-4">
          <p className="break-all">
            成员：<strong>{result.credential.subject}</strong>
          </p>
          <p className="mono break-all">令牌 ID：{result.credential.id}</p>
          {result.token ? (
            <>
              <Label htmlFor="issued-token">个人访问令牌（仅展示一次）</Label>
              <Input
                id="issued-token"
                readOnly
                value={result.token}
                className="mono"
                autoComplete="off"
              />
              <Button
                variant="outline"
                onClick={() => {
                  void navigator.clipboard
                    .writeText(result.token!)
                    .then(() => {
                      setCopied(true);
                      setCopyError("");
                    })
                    .catch(() =>
                      setCopyError("复制失败，请手动选择并复制令牌。"),
                    );
                }}
              >
                {copied ? "已复制" : "复制令牌"}
              </Button>
              <ErrorBox message={copyError} />
              <div className="flex items-start gap-3">
                <Checkbox
                  id="token-saved"
                  checked={saved}
                  onCheckedChange={(value) => setSaved(value === true)}
                  className="mt-0.5"
                />
                <Label htmlFor="token-saved" className="leading-5">
                  我已保存令牌，知道关闭后无法找回
                </Label>
              </div>
              <Button disabled={!saved} onClick={closeDialog}>
                完成
              </Button>
            </>
          ) : (
            <>
              <Notice tone="muted">
                此前请求已成功，但明文无法再次返回。请撤销此令牌，再签发新令牌；不要把令牌
                ID 当作登录凭据。
              </Notice>
              <Button onClick={closeDialog}>返回列表处理</Button>
            </>
          )}
        </div>
      ) : (
        <form
          className="grid gap-4"
          onSubmit={(e) => {
            e.preventDefault();
            if (pending) {
              submit(pending);
              return;
            }
            const data = new FormData(e.currentTarget);
            const request: Pending = {
              action: previous ? "reissueToken" : "issueToken",
              requestId: crypto.randomUUID(),
              ...(previous ? { tokenId: previous.id } : { subject }),
              label: String(data.get("label")).trim(),
              expiresAt: expiry.toISOString(),
            };
            setPending(request);
            submit(request);
          }}
        >
          <fieldset disabled={!!pending || task.busy} className="grid gap-4">
            {!previous && (
              <div className="field">
                <Label htmlFor="token-subject">成员身份</Label>
                <Select
                  name="subject"
                  required
                  value={subject}
                  onValueChange={setSubject}
                  disabled={membersBusy || !!pending || task.busy}
                >
                  <SelectTrigger id="token-subject" className="w-full">
                    <SelectValue
                      placeholder={
                        membersBusy ? "正在加载项目成员…" : "选择现有项目成员"
                      }
                    />
                  </SelectTrigger>
                  <SelectContent>
                    {members.map((member) => (
                      <SelectItem key={member.subject} value={member.subject}>
                        {member.subject}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                <ErrorBox message={membersError} />
                {membersError && (
                  <Button
                    type="button"
                    variant="outline"
                    onClick={() => setRetryMembers((n) => n + 1)}
                  >
                    重试加载成员
                  </Button>
                )}
                {!membersBusy && !membersError && !members.length && (
                  <Notice tone="muted">
                    当前项目没有成员，请先到访问权限中添加。
                  </Notice>
                )}
              </div>
            )}
            <div className="field">
              <Label htmlFor="token-label">用途标签</Label>
              <Input
                id="token-label"
                name="label"
                maxLength={256}
                defaultValue={previous?.label || ""}
                placeholder="例如 日常工作"
              />
            </div>
            <div className="field">
              <Label htmlFor="token-expiry">到期时间（本地时间）</Label>
              <div className="token-expiry">
                <Popover open={calendarOpen} onOpenChange={setCalendarOpen}>
                  <PopoverTrigger asChild>
                    <Button
                      id="token-expiry"
                      type="button"
                      variant="outline"
                      className="justify-start"
                      disabled={!!pending || task.busy}
                    >
                      {expiry.toLocaleDateString("zh-CN", {
                        year: "numeric",
                        month: "long",
                        day: "numeric",
                      })}
                    </Button>
                  </PopoverTrigger>
                  <PopoverContent
                    className="w-auto p-0"
                    align="start"
                    onOpenAutoFocus={(event) => {
                      event.preventDefault();
                      (event.target as HTMLElement)
                        .querySelector<HTMLButtonElement>(
                          "[data-selected-single=true]",
                        )
                        ?.focus();
                    }}
                  >
                    <Calendar
                      autoFocus
                      mode="single"
                      required
                      locale={zhCN}
                      selected={expiry}
                      defaultMonth={expiry}
                      disabled={{
                        before: new Date(new Date().setHours(0, 0, 0, 0)),
                      }}
                      onSelect={(day) => {
                        const next = new Date(day);
                        next.setHours(
                          expiry.getHours(),
                          expiry.getMinutes(),
                          0,
                          0,
                        );
                        setExpiry(next);
                        setCalendarOpen(false);
                      }}
                    />
                  </PopoverContent>
                </Popover>
                {(["小时", "分钟"] as const).map((label, index) => (
                  <Select
                    key={label}
                    disabled={!!pending || task.busy}
                    value={String(
                      index ? expiry.getMinutes() : expiry.getHours(),
                    )}
                    onValueChange={(value) => {
                      const next = new Date(expiry);
                      if (index) next.setMinutes(Number(value));
                      else next.setHours(Number(value));
                      next.setSeconds(0, 0);
                      setExpiry(next);
                    }}
                  >
                    <SelectTrigger aria-label={label}>
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {Array.from({ length: index ? 60 : 24 }, (_, n) => (
                        <SelectItem key={n} value={String(n)}>
                          {String(n).padStart(2, "0")}
                          {index ? " 分" : " 时"}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                ))}
              </div>
            </div>
          </fieldset>
          <ErrorBox message={task.error} />
          {pending && !task.busy && (
            <Notice tone="muted">
              请重试原请求确认结果。若离开，可在令牌列表查找并撤销未保存的令牌；不要重复签发。
            </Notice>
          )}
          <div className="form-actions">
            <Button
              type="button"
              variant="outline"
              disabled={task.busy}
              onClick={closeDialog}
            >
              取消
            </Button>
            <Button
              disabled={
                task.busy ||
                (!previous && (!subject || membersBusy || !!membersError))
              }
            >
              {task.busy
                ? "正在签发…"
                : pending
                  ? "重试原请求"
                  : previous
                    ? "确认重新签发"
                    : "确认签发"}
            </Button>
          </div>
        </form>
      )}
    </Modal>
  );
}

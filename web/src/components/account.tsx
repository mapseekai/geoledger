"use client";
import { useRouter } from "next/navigation";
import { keyExpiryInput, keyExpiryChange } from "@/lib/api-key-expiry";
import { useEffect, useState, type FormEvent } from "react";
import type { User, ApiKey } from "@/lib/auth-model";
import { PasswordInput } from "./password-input";
import { validatePasswordForm } from "@/lib/password";
import { send } from "@/lib/browser-api";
import { Button } from "./ui/button";
import { Input } from "./ui/input";
import { Label } from "./ui/label";
import { Checkbox } from "./ui/checkbox";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "./ui/table";
import { ErrorBox, Modal } from "./common";
import { Panel, PanelTitle, useAction, time } from "./resource-shared";

function auth<T>(action: string, input: Record<string, unknown> = {}) {
  return send<T>("/api/auth", "POST", { action, ...input });
}
function PasswordField({
  label = "当前密码",
  name = "current_password",
}: {
  label?: string;
  name?: string;
}) {
  return (
    <div className="field">
      <Label htmlFor={name}>{label}</Label>
      <PasswordInput
        id={name}
        name={name}
        type="password"
        autoComplete={
          name === "current_password" ? "current-password" : "new-password"
        }
        required
        minimum={name === "current_password" ? 1 : 12}
      />
      <small>UTF-8 {name === "current_password" ? 1 : 12}–1024 字节</small>
    </div>
  );
}
export function AccountPanel({
  section,
  user,
}: {
  section: string;
  user: User;
}) {
  if (section === "users") return <UsersPanel />;
  if (section === "api-keys") return <KeysPanel user={user} />;
  return <ProfilePanel user={user} password={section === "password"} />;
}
function ProfilePanel({ user, password }: { user: User; password: boolean }) {
  const action = useAction();
  const router = useRouter();
  const [profile, setProfile] = useState(user);
  const [message, setMessage] = useState("");
  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!validatePasswordForm(event.currentTarget)) return;
    const form = event.currentTarget;
    const data = new FormData(form);
    void action.run(async () => {
      if (password) {
        if (data.get("new_password") !== data.get("confirm_password"))
          throw new Error("两次输入的新密码不一致。");
        await auth("change_password", {
          current_password: data.get("current_password"),
          new_password: data.get("new_password"),
        });
        form.reset();
        window.location.replace("/projects");
      } else {
        const updated = await auth<User>("update_user", {
          user_id: user.id,
          username: data.get("username"),
          display_name: data.get("display_name"),
          current_password: data.get("current_password"),
        });
        setProfile(updated);
        form.reset();
        setMessage("个人资料已更新。");
        router.refresh();
      }
    });
  };
  return (
    <Panel
      toolbar={
        <PanelTitle
          title={password ? "修改密码" : "个人资料"}
          description={
            user.must_change_password
              ? "请先修改初始或重置密码，才能继续使用控制台。"
              : "账户身份由固定用户 ID 标识，修改用户名不会改变项目权限。"
          }
        />
      }
    >
      <form className="card-form" onSubmit={submit}>
        {!password && (
          <>
            <div className="field">
              <Label htmlFor="user-id">用户 ID</Label>
              <Input id="user-id" value={user.id} readOnly />
            </div>
            <div className="field">
              <Label htmlFor="username">用户名</Label>
              <Input
                id="username"
                name="username"
                value={profile.username}
                onChange={(event) =>
                  setProfile({ ...profile, username: event.target.value })
                }
                required
                maxLength={64}
              />
            </div>
            <div className="field">
              <Label htmlFor="display_name">显示名称</Label>
              <Input
                id="display_name"
                name="display_name"
                value={profile.display_name}
                onChange={(event) =>
                  setProfile({ ...profile, display_name: event.target.value })
                }
                maxLength={256}
              />
            </div>
          </>
        )}
        <PasswordField />
        {password && (
          <>
            <PasswordField label="新密码" name="new_password" />
            <PasswordField label="确认新密码" name="confirm_password" />
          </>
        )}
        <ErrorBox message={action.error} />
        <p role="status">{message}</p>
        <Button type="submit" disabled={action.busy}>
          {action.busy ? "保存中…" : "保存"}
        </Button>
      </form>
    </Panel>
  );
}
type UserEdit = {
  user?: User;
  kind: "create" | "edit" | "reset" | "disable" | "enable" | "delete";
};
function UsersPanel() {
  const [rows, setRows] = useState<User[]>([]),
    [after, setAfter] = useState<string>(),
    [status, setStatus] = useState(""),
    [edit, setEdit] = useState<UserEdit>(),
    [loadError, setLoadError] = useState(""),
    [loading, setLoading] = useState(true),
    [reload, setReload] = useState(0);
  const write = useAction();
  const refresh = () => setReload((value) => value + 1);
  useEffect(() => {
    let active = true;
    setRows([]);
    setLoading(true);
    setLoadError("");
    void auth<User[]>("list_users", {
      after,
      limit: 20,
      ...(status ? { status } : {}),
    })
      .then((v) => {
        if (active) {
          setRows(v);
          setLoadError("");
        }
      })
      .catch((e) => {
        if (active) setLoadError(e instanceof Error ? e.message : "加载失败");
      })
      .finally(() => {
        if (active) setLoading(false);
      });
    return () => {
      active = false;
    };
  }, [after, status, reload]);
  function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!validatePasswordForm(event.currentTarget)) return;
    const data = new FormData(event.currentTarget);
    if (!edit) return;
    void write.run(async () => {
      const input: Record<string, unknown> = {
        current_password: data.get("current_password"),
      };
      if (edit.user) input.user_id = edit.user.id;
      if (edit.kind === "create" || edit.kind === "edit")
        Object.assign(input, {
          username: data.get("username"),
          display_name: data.get("display_name"),
          is_admin: data.get("is_admin") === "on",
        });
      if (edit.kind === "create") input.password = data.get("new_password");
      if (edit.kind === "reset") input.new_password = data.get("new_password");
      if (["disable", "enable", "delete"].includes(edit.kind))
        input.status =
          edit.kind === "disable"
            ? "disabled"
            : edit.kind === "delete"
              ? "deleted"
              : "active";
      await auth(
        edit.kind === "create"
          ? "create_user"
          : edit.kind === "reset"
            ? "reset_password"
            : "update_user",
        input,
      );
      setEdit(undefined);
      refresh();
    });
  }
  const title =
    edit?.kind === "create"
      ? "新增用户"
      : edit?.kind === "reset"
        ? "重置密码"
        : edit?.kind === "delete"
          ? "删除用户"
          : edit?.kind === "disable"
            ? "禁用用户"
            : edit?.kind === "enable"
              ? "启用用户"
              : "编辑用户";
  return (
    <>
      <Panel
        toolbar={
          <>
            <PanelTitle
              title="用户管理"
              description="禁用、删除和重置密码会立即使相关凭证失效。"
            />
            <Button onClick={() => setEdit({ kind: "create" })}>
              新增用户
            </Button>
          </>
        }
      >
        <div className="panel-toolbar">
          <Button variant="outline" onClick={refresh}>
            刷新
          </Button>
          {["", "active", "disabled", "deleted"].map((v, i) => (
            <Button
              key={v}
              variant={status === v ? "secondary" : "ghost"}
              onClick={() => {
                setStatus(v);
                setAfter(undefined);
              }}
            >
              {["全部", "启用", "禁用", "已删除"][i]}
            </Button>
          ))}
        </div>
        <ErrorBox message={loadError} />
        {loading && <p role="status">加载中…</p>}
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>用户</TableHead>
              <TableHead>状态</TableHead>
              <TableHead>身份</TableHead>
              <TableHead>操作</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {rows.map((u) => (
              <TableRow key={u.id}>
                <TableCell>
                  <strong>{u.username}</strong>
                  <p>{u.display_name}</p>
                  <small className="mono">{u.id}</small>
                </TableCell>
                <TableCell>
                  {u.status === "active"
                    ? "启用"
                    : u.status === "disabled"
                      ? "禁用"
                      : "已删除"}
                  {u.must_change_password ? " · 待改密" : ""}
                </TableCell>
                <TableCell>{u.is_admin ? "管理员" : "普通用户"}</TableCell>
                <TableCell>
                  {u.status !== "deleted" && (
                    <>
                      <Button
                        variant="ghost"
                        onClick={() => setEdit({ kind: "edit", user: u })}
                      >
                        编辑
                      </Button>
                      <Button
                        variant="ghost"
                        onClick={() => setEdit({ kind: "reset", user: u })}
                      >
                        重置密码
                      </Button>
                      <Button
                        variant="ghost"
                        onClick={() =>
                          setEdit({
                            kind: u.status === "active" ? "disable" : "enable",
                            user: u,
                          })
                        }
                      >
                        {u.status === "active" ? "禁用" : "启用"}
                      </Button>
                      <Button
                        variant="ghost"
                        onClick={() => setEdit({ kind: "delete", user: u })}
                      >
                        删除
                      </Button>
                    </>
                  )}
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
        <div className="panel-toolbar">
          <Button
            variant="outline"
            disabled={!after}
            onClick={() => setAfter(undefined)}
          >
            首页
          </Button>
          <Button
            variant="outline"
            disabled={rows.length < 20}
            onClick={() => setAfter(rows.at(-1)?.id)}
          >
            下一页
          </Button>
        </div>
      </Panel>
      {edit && (
        <Modal title={title} busy={write.busy} close={() => setEdit(undefined)}>
          <form className="card-form" onSubmit={submit}>
            <p>
              {edit.user?.username}
              {["delete", "disable"].includes(edit.kind)
                ? "：确认此操作将立即终止该用户访问，历史审计仍保留。"
                : ""}
            </p>
            {["create", "edit"].includes(edit.kind) && (
              <>
                <div className="field">
                  <Label htmlFor="username">用户名</Label>
                  <Input
                    id="username"
                    name="username"
                    defaultValue={edit.user?.username}
                    required
                    maxLength={64}
                  />
                </div>
                <div className="field">
                  <Label htmlFor="display_name">显示名称</Label>
                  <Input
                    id="display_name"
                    name="display_name"
                    defaultValue={edit.user?.display_name}
                    maxLength={256}
                  />
                </div>
                <Label>
                  <Checkbox
                    name="is_admin"
                    defaultChecked={edit.user?.is_admin}
                  />
                  平台管理员
                </Label>
              </>
            )}
            {["create", "reset"].includes(edit.kind) && (
              <PasswordField
                label="临时密码（用户首次登录必须修改）"
                name="new_password"
              />
            )}
            <PasswordField label="你的当前密码" />
            <ErrorBox message={write.error} />
            <Button type="submit" disabled={write.busy}>
              确认{title}
            </Button>
          </form>
        </Modal>
      )}
    </>
  );
}
type KeyEdit = { key?: ApiKey; kind: "create" | "edit" | "revoke" | "delete" };
function KeysPanel({ user }: { user: User }) {
  const [rows, setRows] = useState<ApiKey[]>([]),
    [owner, setOwner] = useState(user.id),
    [after, setAfter] = useState<string>(),
    [edit, setEdit] = useState<KeyEdit>(),
    [secret, setSecret] = useState<string>(),
    [copied, setCopied] = useState(false),
    [loadError, setLoadError] = useState(""),
    [loading, setLoading] = useState(true),
    [reload, setReload] = useState(0);
  const write = useAction();
  const refresh = () => setReload((value) => value + 1);
  useEffect(() => {
    let active = true;
    setRows([]);
    setLoading(true);
    setLoadError("");
    void auth<ApiKey[]>("list_api_keys", { user_id: owner, after, limit: 20 })
      .then((v) => {
        if (active) {
          setRows(v);
          setLoadError("");
        }
      })
      .catch((e) => {
        if (active) setLoadError(e instanceof Error ? e.message : "加载失败");
      })
      .finally(() => {
        if (active) setLoading(false);
      });
    return () => {
      active = false;
    };
  }, [owner, after, reload]);
  function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!validatePasswordForm(event.currentTarget)) return;
    const data = new FormData(event.currentTarget);
    if (!edit) return;
    void write.run(async () => {
      const input: Record<string, unknown> = {
        current_password: data.get("current_password"),
      };
      if (edit.key) input.key_id = edit.key.id;
      else input.user_id = owner;
      if (["create", "edit"].includes(edit.kind)) {
        const expiry = String(data.get("expires_at") ?? "");
        Object.assign(input, {
          name: data.get("name"),
          expires_at: keyExpiryChange(edit.key?.expires_at, expiry),
        });
      }
      const result = await auth<{ api_key: string }>(
        edit.kind === "create"
          ? "create_api_key"
          : edit.kind === "edit"
            ? "update_api_key"
            : edit.kind === "revoke"
              ? "revoke_api_key"
              : "delete_api_key",
        input,
      );
      if (edit.kind === "create") {
        setSecret(result.api_key);
        setCopied(false);
      }
      setEdit(undefined);
      refresh();
    });
  }
  const title =
    edit?.kind === "create"
      ? "创建 API Key"
      : edit?.kind === "edit"
        ? "编辑 API Key"
        : edit?.kind === "revoke"
          ? "撤销 API Key"
          : "删除 API Key";
  return (
    <>
      <Panel
        toolbar={
          <>
            <PanelTitle
              title="API Key"
              description="多个密钥可同时使用。新建密钥不会撤销现有密钥；权限始终属于绑定用户。"
            />
            <Button onClick={() => setEdit({ kind: "create" })}>
              创建 API Key
            </Button>
          </>
        }
      >
        <div className="panel-toolbar">
          {user.is_admin && (
            <form
              onSubmit={(e) => {
                e.preventDefault();
                setOwner(String(new FormData(e.currentTarget).get("user_id")));
                setAfter(undefined);
              }}
            >
              <Label htmlFor="key-owner">用户 ID</Label>
              <Input
                id="key-owner"
                name="user_id"
                defaultValue={owner}
                required
              />
              <Button variant="outline" type="submit">
                查看用户密钥
              </Button>
            </form>
          )}
          <Button variant="outline" onClick={refresh}>
            刷新
          </Button>
        </div>
        <ErrorBox message={loadError} />
        {loading && <p role="status">加载中…</p>}
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>名称 / 前缀</TableHead>
              <TableHead>状态</TableHead>
              <TableHead>用户 / 创建时间</TableHead>
              <TableHead>有效期</TableHead>
              <TableHead>最近使用</TableHead>
              <TableHead>操作</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {rows.map((k) => (
              <TableRow key={k.id}>
                <TableCell>
                  {k.name}
                  <p className="mono">{k.key_prefix}…</p>
                  <small>{k.id}</small>
                </TableCell>
                <TableCell>
                  {k.revoked_at
                    ? "已撤销"
                    : k.expires_at && Date.parse(k.expires_at) <= Date.now()
                      ? "已过期"
                      : "有效"}
                  {k.revoked_at && <p>{time(k.revoked_at)}</p>}
                </TableCell>
                <TableCell>
                  <span className="mono">{k.user_id}</span>
                  <p>{time(k.created_at)}</p>
                </TableCell>
                <TableCell>
                  {k.expires_at ? time(k.expires_at) : "无到期时间"}
                </TableCell>
                <TableCell>
                  {k.last_used_at ? time(k.last_used_at) : "尚未使用"}
                </TableCell>
                <TableCell>
                  {!k.revoked_at && (
                    <>
                      <Button
                        variant="ghost"
                        onClick={() => setEdit({ kind: "edit", key: k })}
                      >
                        编辑
                      </Button>
                      <Button
                        variant="ghost"
                        onClick={() => setEdit({ kind: "revoke", key: k })}
                      >
                        撤销
                      </Button>
                    </>
                  )}
                  <Button
                    variant="ghost"
                    onClick={() => setEdit({ kind: "delete", key: k })}
                  >
                    删除
                  </Button>
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
        <div className="panel-toolbar">
          <Button
            variant="outline"
            disabled={!after}
            onClick={() => setAfter(undefined)}
          >
            首页
          </Button>
          <Button
            variant="outline"
            disabled={rows.length < 20}
            onClick={() => setAfter(rows.at(-1)?.id)}
          >
            下一页
          </Button>
        </div>
      </Panel>
      {edit && (
        <Modal title={title} busy={write.busy} close={() => setEdit(undefined)}>
          <form className="card-form" onSubmit={submit}>
            <p>
              {edit.key?.name}
              {["revoke", "delete"].includes(edit.kind)
                ? "：确认立即停止此密钥及由它创建的控制台会话访问。"
                : ""}
            </p>
            {["create", "edit"].includes(edit.kind) && (
              <>
                <div className="field">
                  <Label htmlFor="name">名称</Label>
                  <Input
                    id="name"
                    name="name"
                    defaultValue={edit.key?.name}
                    required
                    maxLength={256}
                  />
                </div>
                <div className="field">
                  <Label htmlFor="expires_at">
                    到期时间（UTC，留空为无到期时间）
                  </Label>
                  <Input
                    id="expires_at"
                    name="expires_at"
                    type="datetime-local"
                    step="1"
                    defaultValue={keyExpiryInput(edit.key?.expires_at)}
                  />
                </div>
              </>
            )}
            <PasswordField />
            <ErrorBox message={write.error} />
            <Button type="submit" disabled={write.busy}>
              确认{title}
            </Button>
          </form>
        </Modal>
      )}
      {secret && (
        <Modal title="保存 API Key" close={() => setSecret(undefined)}>
          <p>密钥仅显示这一次，请立即复制并安全保存。关闭后无法再次查看。</p>
          <Label htmlFor="new-key">新密钥</Label>
          <Input id="new-key" value={secret} readOnly autoComplete="off" />
          <Button
            disabled={write.busy}
            onClick={() =>
              void write.run(async () => {
                await navigator.clipboard.writeText(secret);
                setCopied(true);
              })
            }
          >
            {copied ? "已复制" : "复制密钥"}
          </Button>
          <ErrorBox message={write.error} />
          <Button variant="outline" onClick={() => setSecret(undefined)}>
            我已安全保存
          </Button>
        </Modal>
      )}
    </>
  );
}

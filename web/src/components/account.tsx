"use client";
import { FieldGroup } from "./ui/field";
import { ToggleGroup, ToggleGroupItem } from "./ui/toggle-group";
import { Field } from "./ui/field";
import { FieldLabel } from "./ui/field";
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
import { Empty, ErrorBox, IdText, Modal } from "./common";
import { useUnsavedChanges } from "./unsaved-changes";
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
    <Field>
      <FieldLabel htmlFor={name}>{label}</FieldLabel>
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
    </Field>
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
  const [dirty, setDirty] = useState(false);
  const unsaved = useUnsavedChanges(dirty);
  const [message, setMessage] = useState("");
  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!validatePasswordForm(event.currentTarget)) return;
    const form = event.currentTarget;
    const data = new FormData(form);
    // oxlint-disable-next-line typescript/no-floating-promises -- useAction.run catches failures and reports them in component state.
    void action.run(async () => {
      if (password) {
        if (data.get("new_password") !== data.get("confirm_password"))
          throw new Error("两次输入的新密码不一致。");
        await auth("change_password", {
          current_password: data.get("current_password"),
          new_password: data.get("new_password"),
        });
        unsaved.markSaved();
        setDirty(false);
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
        unsaved.markSaved();
        setDirty(false);
        form.reset();
        setMessage("个人资料已更新。");
        router.refresh();
      }
    });
  };
  return (
    <>
      <Panel
        className="account-profile"
        toolbar={
          <PanelTitle
            title={password ? "修改密码" : "个人资料"}
            description={
              user.must_change_password
                ? "请先修改初始或重置密码，才能继续使用控制台。"
                : password
                  ? "使用至少 12 字节的新密码，修改后其他会话将失效。"
                  : "账户身份由固定用户 ID 标识，修改用户名不会改变项目权限。"
            }
          />
        }
      >
        <form
          className="card-form"
          onChange={() => setDirty(true)}
          onSubmit={submit}
        >
          <FieldGroup>
            {!password && (
              <>
                <Field>
                  <FieldLabel htmlFor="user-id">用户 ID</FieldLabel>
                  <Input id="user-id" value={user.id} readOnly />
                </Field>
                <Field>
                  <FieldLabel htmlFor="username">用户名</FieldLabel>
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
                </Field>
                <Field>
                  <FieldLabel htmlFor="display_name">显示名称</FieldLabel>
                  <Input
                    id="display_name"
                    name="display_name"
                    value={profile.display_name}
                    onChange={(event) =>
                      setProfile({
                        ...profile,
                        display_name: event.target.value,
                      })
                    }
                    maxLength={256}
                  />
                </Field>
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
            <p role="status" className={message ? undefined : "sr-only"}>
              {message}
            </p>
            <div className="form-actions">
              <Button type="submit" disabled={action.busy}>
                {action.busy ? "保存中…" : "保存"}
              </Button>
            </div>
          </FieldGroup>
        </form>
      </Panel>
      {unsaved.confirmation}
    </>
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
    // oxlint-disable-next-line typescript/no-floating-promises -- useAction.run catches failures and reports them in component state.
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
          <ToggleGroup
            type="single"
            value={status || "all"}
            aria-label="用户状态"
            onValueChange={(value) => {
              if (value) {
                setStatus(value === "all" ? "" : value);
                setAfter(undefined);
              }
            }}
          >
            {["all", "active", "disabled", "deleted"].map((value, i) => (
              <ToggleGroupItem key={value} value={value}>
                {["全部", "启用", "禁用", "已删除"][i]}
              </ToggleGroupItem>
            ))}
          </ToggleGroup>
          <Button variant="outline" onClick={refresh}>
            刷新
          </Button>
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
            {!loading && !loadError && rows.length === 0 && (
              <TableRow>
                <TableCell colSpan={4}>
                  <Empty title={status ? "没有匹配的用户" : "暂无用户"}>
                    可切换状态筛选或新增用户。
                  </Empty>
                </TableCell>
              </TableRow>
            )}
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
            <FieldGroup>
              <p>
                {edit.user?.username}
                {["delete", "disable"].includes(edit.kind)
                  ? "：确认此操作将立即终止该用户访问，历史审计仍保留。"
                  : ""}
              </p>
              {["create", "edit"].includes(edit.kind) && (
                <>
                  <Field>
                    <FieldLabel htmlFor="username">用户名</FieldLabel>
                    <Input
                      id="username"
                      name="username"
                      defaultValue={edit.user?.username}
                      required
                      maxLength={64}
                    />
                  </Field>
                  <Field>
                    <FieldLabel htmlFor="display_name">显示名称</FieldLabel>
                    <Input
                      id="display_name"
                      name="display_name"
                      defaultValue={edit.user?.display_name}
                      maxLength={256}
                    />
                  </Field>
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
            </FieldGroup>
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
    // oxlint-disable-next-line typescript/no-floating-promises -- useAction.run catches failures and reports them in component state.
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
              className="account-key-lookup"
              onSubmit={(e) => {
                e.preventDefault();
                setOwner(String(new FormData(e.currentTarget).get("user_id")));
                setAfter(undefined);
              }}
            >
              <Field>
                <FieldLabel htmlFor="key-owner">用户 ID</FieldLabel>
                <Input
                  id="key-owner"
                  name="user_id"
                  defaultValue={owner}
                  required
                />
              </Field>
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
        <div className="account-keys-table">
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
              {!loading && !loadError && rows.length === 0 && (
                <TableRow>
                  <TableCell colSpan={6}>
                    <Empty title="暂无 API Key">
                      该用户还没有密钥，可创建密钥后使用。
                    </Empty>
                  </TableCell>
                </TableRow>
              )}
              {rows.map((k) => (
                <TableRow key={k.id}>
                  <TableCell data-label="名称 / 前缀">
                    {k.name}
                    <p className="mono">{k.key_prefix}…</p>
                    <IdText value={k.id} />
                  </TableCell>
                  <TableCell data-label="状态">
                    {k.revoked_at
                      ? "已撤销"
                      : k.expires_at && Date.parse(k.expires_at) <= Date.now()
                        ? "已过期"
                        : "有效"}
                    {k.revoked_at && <p>{time(k.revoked_at)}</p>}
                  </TableCell>
                  <TableCell data-label="用户 / 创建时间">
                    <IdText value={k.user_id} />
                    <p>{time(k.created_at)}</p>
                  </TableCell>
                  <TableCell data-label="有效期">
                    {k.expires_at ? time(k.expires_at) : "无到期时间"}
                  </TableCell>
                  <TableCell data-label="最近使用">
                    {k.last_used_at ? time(k.last_used_at) : "尚未使用"}
                  </TableCell>
                  <TableCell data-label="操作">
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
        </div>
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
            <FieldGroup>
              <p>
                {edit.key?.name}
                {["revoke", "delete"].includes(edit.kind)
                  ? "：确认立即停止此密钥及由它创建的控制台会话访问。"
                  : ""}
              </p>
              {["create", "edit"].includes(edit.kind) && (
                <>
                  <Field>
                    <FieldLabel htmlFor="name">名称</FieldLabel>
                    <Input
                      id="name"
                      name="name"
                      defaultValue={edit.key?.name}
                      required
                      maxLength={256}
                    />
                  </Field>
                  <Field>
                    <FieldLabel htmlFor="expires_at">
                      到期时间（UTC，留空为无到期时间）
                    </FieldLabel>
                    <Input
                      id="expires_at"
                      name="expires_at"
                      type="datetime-local"
                      step="1"
                      defaultValue={keyExpiryInput(edit.key?.expires_at)}
                    />
                  </Field>
                </>
              )}
              <PasswordField />
              <ErrorBox message={write.error} />
              <Button type="submit" disabled={write.busy}>
                确认{title}
              </Button>
            </FieldGroup>
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

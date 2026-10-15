"use client";
import { Card } from "./ui/card";
import { FieldGroup } from "./ui/field";
import { Field } from "./ui/field";
import { FieldLabel } from "./ui/field";
import {
  InputGroup,
  InputGroupInput,
  InputGroupAddon,
  InputGroupButton,
} from "./ui/input-group";
import { validatePasswordForm, passwordError } from "@/lib/password";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Tabs, TabsList, TabsTrigger, TabsContent } from "./ui/tabs";
import { send } from "@/lib/browser-api";
import {
  ArrowRight,
  Eye,
  EyeOff,
  GitMerge,
  History,
  LockKeyhole,
  ShieldCheck,
} from "lucide-react";
import { useState, type FormEvent } from "react";
import { Brand } from "./brand";
import { ErrorBox, Tip } from "./common";
const highlights = [
  {
    icon: GitMerge,
    title: "多人协作编辑",
  },
  {
    icon: History,
    title: "版本发布与撤销",
  },
  {
    icon: ShieldCheck,
    title: "权限与审计",
  },
];
export function Login({ expired }: { expired: boolean }) {
  const [mode, setMode] = useState<"password" | "key">("password");
  const [show, setShow] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState(expired ? "登录已过期，请重新登录。" : "");
  async function login(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!validatePasswordForm(event.currentTarget)) return;
    setBusy(true);
    setError("");
    const form = event.currentTarget;
    try {
      const fields = new FormData(form);
      const result = await send<{ user: { must_change_password: boolean } }>(
        "/api/session",
        "POST",
        mode === "key"
          ? { api_key: String(fields.get("api_key") ?? "").trim() }
          : {
              username: String(fields.get("username") ?? ""),
              password: String(fields.get("password") ?? ""),
            },
      );
      form.reset();
      window.location.replace(
        result.user.must_change_password ? "/password" : "/projects",
      );
    } catch (error) {
      setError(error instanceof Error ? error.message : "登录失败");
      setBusy(false);
    }
  }
  return (
    <main className="login-page">
      <section className="login-story" aria-label="GeoLedger 简介">
        <div className="login-map" aria-hidden="true" />
        <Brand caption="管理控制台" />
        <div className="story-copy">
          <span className="story-eyebrow">空间数据版本控制</span>
          <h1>
            每一次改变，
            <br />
            都有迹可循。
          </h1>
          <ul className="story-points">
            {highlights.map((h) => (
              <li key={h.title}>
                <span className="story-icon" aria-hidden="true">
                  <h.icon />
                </span>
                <strong>{h.title}</strong>
              </li>
            ))}
          </ul>
        </div>
      </section>
      <section className="login-panel">
        <div className="mobile-brand">
          <Brand caption="管理控制台" />
        </div>
        <Card className="login-card">
          <form onSubmit={login} className="login-form">
            <span className="login-icon" aria-hidden="true">
              <LockKeyhole />
            </span>
            <h2>登录控制台</h2>
            <Tabs
              value={mode}
              onValueChange={(value) => {
                setMode(value === "key" ? "key" : "password");
                setShow(false);
                setError("");
              }}
            >
              <TabsList aria-label="登录方式" className="w-full mb-4">
                <TabsTrigger value="password" disabled={busy}>
                  密码登录
                </TabsTrigger>
                <TabsTrigger value="key" disabled={busy}>
                  API Key 登录
                </TabsTrigger>
              </TabsList>
              {(["password", "key"] as const).map((loginMode) => (
                <TabsContent key={loginMode} value={loginMode}>
                  <FieldGroup>
                    {loginMode === "password" && (
                      <Field>
                        <FieldLabel htmlFor="username">用户名</FieldLabel>
                        <Input
                          id="username"
                          name="username"
                          autoComplete="username"
                          required
                          maxLength={128}
                          disabled={busy}
                        />
                      </Field>
                    )}
                    <Field>
                      <FieldLabel htmlFor="credential">
                        {loginMode === "password" ? "密码" : "API Key"}
                      </FieldLabel>
                      <InputGroup>
                        <InputGroupInput
                          data-password-min={1}
                          onInput={(event) =>
                            event.currentTarget.setCustomValidity(
                              passwordError(event.currentTarget.value, 1),
                            )
                          }
                          key={loginMode}
                          id="credential"
                          name={
                            loginMode === "password" ? "password" : "api_key"
                          }
                          type={show ? "text" : "password"}
                          autoComplete={
                            loginMode === "password"
                              ? "current-password"
                              : "off"
                          }
                          required
                          placeholder={
                            loginMode === "password" ? "输入密码" : "glk_…"
                          }
                          disabled={busy}
                        />
                        <InputGroupAddon align="inline-end">
                          <Tip label={show ? "隐藏凭证" : "显示凭证"}>
                            <InputGroupButton
                              type="button"
                              variant="ghost"
                              size="icon-sm"
                              aria-label={show ? "隐藏凭证" : "显示凭证"}
                              onClick={() => setShow(!show)}
                            >
                              {show ? <EyeOff /> : <Eye />}
                            </InputGroupButton>
                          </Tip>
                        </InputGroupAddon>
                      </InputGroup>
                    </Field>
                    {loginMode === "password" && (
                      <small>密码：UTF-8 1–1024 字节</small>
                    )}
                  </FieldGroup>
                </TabsContent>
              ))}
            </Tabs>
            <ErrorBox message={error} />
            <Button className="login-submit" disabled={busy} type="submit">
              {busy ? "正在连接…" : "进入控制台"}
              <ArrowRight />
            </Button>
          </form>
        </Card>
      </section>
    </main>
  );
}

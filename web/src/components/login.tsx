"use client";
import { PasswordInput } from "./password-input";
import { validatePasswordForm } from "@/lib/password";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
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
        <div className="login-card">
          <form onSubmit={login} className="login-form">
            <span className="login-icon" aria-hidden="true">
              <LockKeyhole />
            </span>
            <h2>登录控制台</h2>
            <Button
              type="button"
              variant="outline"
              disabled={busy}
              onClick={() => {
                setMode(mode === "password" ? "key" : "password");
                setShow(false);
              }}
            >
              {mode === "password"
                ? "使用 API Key 登录"
                : "使用用户名和密码登录"}
            </Button>
            {mode === "password" && (
              <div className="field">
                <Label htmlFor="username">用户名</Label>
                <Input
                  id="username"
                  name="username"
                  autoComplete="username"
                  required
                  maxLength={128}
                  disabled={busy}
                />
              </div>
            )}
            <div className="field">
              <Label htmlFor="credential">
                {mode === "password" ? "密码" : "API Key"}
              </Label>
              <div className="password-field">
                <PasswordInput
                  key={mode}
                  id="credential"
                  name={mode === "password" ? "password" : "api_key"}
                  type={show ? "text" : "password"}
                  autoComplete={
                    mode === "password" ? "current-password" : "off"
                  }
                  required
                  placeholder={mode === "password" ? "输入密码" : "glk_…"}
                  disabled={busy}
                />
                <Tip label={show ? "隐藏凭证" : "显示凭证"}>
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon-sm"
                    aria-label={show ? "隐藏凭证" : "显示凭证"}
                    onClick={() => setShow(!show)}
                  >
                    {show ? <EyeOff /> : <Eye />}
                  </Button>
                </Tip>
              </div>
            </div>
            {mode === "password" && <small>密码：UTF-8 1–1024 字节</small>}
            <ErrorBox message={error} />
            <Button className="login-submit" disabled={busy} type="submit">
              {busy ? "正在连接…" : "进入控制台"}
              <ArrowRight />
            </Button>
          </form>
        </div>
      </section>
    </main>
  );
}

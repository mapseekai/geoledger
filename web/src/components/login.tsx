"use client";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { send } from "@/lib/browser-api";
import {
  ArrowRight,
  Eye,
  EyeOff,
  GitMerge,
  History,
  KeyRound,
  LockKeyhole,
  ShieldCheck,
} from "lucide-react";
import { useState, type FormEvent } from "react";
import { Brand } from "./brand";
import { ErrorBox } from "./common";
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
  const [show, setShow] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState(expired ? "登录已过期，请重新登录。" : "");
  async function login(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setBusy(true);
    setError("");
    const form = event.currentTarget;
    try {
      await send("/api/session", "POST", {
        token: String(new FormData(form).get("token") ?? "").trim(),
      });
      form.reset();
      window.location.replace("/projects");
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
            <div className="field">
              <label htmlFor="token">访问令牌</label>
              <div className="password-field">
                <KeyRound className="field-icon" aria-hidden="true" />
                <Input
                  id="token"
                  name="token"
                  type={show ? "text" : "password"}
                  autoComplete="off"
                  maxLength={2000}
                  required
                  placeholder="输入访问令牌"
                  disabled={busy}
                />
                <Button
                  type="button"
                  variant="ghost"
                  size="icon-sm"
                  aria-label={show ? "隐藏令牌" : "显示令牌"}
                  onClick={() => setShow(!show)}
                >
                  {show ? <EyeOff /> : <Eye />}
                </Button>
              </div>
            </div>
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

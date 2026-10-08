"use client";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { send } from "@/lib/browser-api";
import { ArrowRight, Eye, EyeOff, LockKeyhole } from "lucide-react";
import { useState, type FormEvent } from "react";
import { Brand, Sunset } from "./brand";
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
      <section className="login-story">
        <Brand />
        <div className="story-copy">
          <h1>
            每一次改变，
            <br />
            都有迹可循。
          </h1>
        </div>
        <div className="landscape" aria-hidden="true">
          <div className="sun" />
          <div className="ridge ridge-back" />
          <div className="ridge ridge-front" />
          <div className="grid-lines" />
          <span className="map-coordinate">35° 00′ N &nbsp; 104° 00′ E</span>
        </div>
        <div className="story-foot">
          <span>空间数据版本控制</span>
          <span>GEОLEDGER CONSOLE</span>
        </div>
      </section>
      <section className="login-panel">
        <div className="mobile-brand">
          <Brand />
        </div>
        <form onSubmit={login} className="login-form">
          <div className="icon-tile">
            <LockKeyhole size={23} />
          </div>
          <h2>登录控制台</h2>
          <label htmlFor="token">访问令牌</label>
          <div className="password-field">
            <Input
              id="token"
              name="token"
              type={show ? "text" : "password"}
              autoComplete="off"
              maxLength={2000}
              required
              placeholder="输入管理员或成员令牌"
              disabled={busy}
            />
            <Button
              type="button"
              variant="ghost"
              size="icon"
              aria-label={show ? "隐藏令牌" : "显示令牌"}
              onClick={() => setShow(!show)}
            >
              {show ? <EyeOff /> : <Eye />}
            </Button>
          </div>
          {error && (
            <div role="alert" className="alert">
              {error}
            </div>
          )}
          <Button className="login-submit" disabled={busy} type="submit">
            {busy ? "正在连接…" : "进入控制台"}
            <ArrowRight />
          </Button>
        </form>
        <p className="login-footer">
          GeoLedger &nbsp; / &nbsp; 空间数据版本控制
        </p>
      </section>
      <Sunset />
    </main>
  );
}

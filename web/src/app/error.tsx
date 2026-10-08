"use client";
import { Button } from "@/components/ui/button";
export default function ErrorPage({ reset }: { reset: () => void }) {
  return (
    <main className="fatal">
      <h1>控制台暂时不可用</h1>
      <p>请检查 Web 服务配置与后端连接后重试。</p>
      <Button onClick={reset}>重试</Button>
      <a href="/login">返回登录</a>
    </main>
  );
}

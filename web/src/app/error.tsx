"use client";
import { Button } from "@/components/ui/button";
import { ServerCrash } from "lucide-react";
export default function ErrorPage({ reset }: { reset: () => void }) {
  return (
    <main className="fatal">
      <div className="fatal-card">
        <span className="fatal-icon" aria-hidden="true">
          <ServerCrash />
        </span>
        <h1>控制台暂时不可用</h1>
        <p>请检查 Web 服务配置与后端连接后重试。</p>
        <div className="fatal-actions">
          <Button onClick={reset}>重试</Button>
          <Button asChild variant="outline">
            <a href="/login">返回登录</a>
          </Button>
        </div>
      </div>
    </main>
  );
}

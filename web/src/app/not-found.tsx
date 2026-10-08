import { Button } from "@/components/ui/button";
import { Compass } from "lucide-react";
import Link from "next/link";
export default function NotFound() {
  return (
    <main className="fatal">
      <div className="fatal-card">
        <span className="fatal-icon" aria-hidden="true">
          <Compass />
        </span>
        <h1>页面不存在</h1>
        <p>请检查地址，或返回项目列表。</p>
        <div className="fatal-actions">
          <Button asChild>
            <Link href="/projects">返回项目列表</Link>
          </Button>
        </div>
      </div>
    </main>
  );
}

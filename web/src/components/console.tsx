"use client";
import { call, send, type Info, type Project } from "@/lib/browser-api";
import {
  ArrowRight,
  ChevronRight,
  Database,
  Folder,
  GitBranch,
  History,
  LayoutGrid,
  LogOut,
  Menu,
  Server,
  Shield,
  Terminal,
  X,
} from "lucide-react";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { useEffect, useState } from "react";
import { Access, AuditPanel } from "./access-audit";
import { Brand, Sunset } from "./brand";
import { Empty, ErrorBox, Loading } from "./common";
import { HistoryPanel } from "./history";
import { Datasets, Projects, ServiceInfo } from "./projects";
import { Panel, role, short, useAction } from "./resource-shared";
import { Badge } from "./ui/badge";
import { Button } from "./ui/button";
import { Workspaces } from "./workspaces";
const sections = [
  {
    id: "projects",
    title: "项目",
    icon: LayoutGrid,
  },
  {
    id: "datasets",
    title: "数据集",
    icon: Database,
  },
  {
    id: "workspaces",
    title: "工作区",
    icon: GitBranch,
  },
  {
    id: "history",
    title: "版本历史",
    icon: History,
  },
  {
    id: "access",
    title: "访问权限",
    icon: Shield,
  },
  {
    id: "audit",
    title: "审计日志",
    icon: Terminal,
  },
  {
    id: "service",
    title: "服务信息",
    icon: Server,
  },
];
export function Console({
  section,
  projectId,
}: {
  section: string;
  projectId: string;
}) {
  const router = useRouter();
  const [info, setInfo] = useState<Info>(),
    [project, setProject] = useState<Project>(),
    [error, setError] = useState("");
  const [mobile, setMobile] = useState(false);
  const logout = useAction();
  useEffect(() => {
    let active = true;
    Promise.all([
      call<Info>({ action: "info" }),
      projectId
        ? call<Project>({ action: "project", project: projectId })
        : Promise.resolve(undefined),
    ])
      .then(([i, p]) => {
        if (active) {
          setInfo(i);
          setProject(p);
        }
      })
      .catch((e) => {
        if (active) setError(e.message);
      });
    return () => {
      active = false;
    };
  }, [projectId]);
  const meta = sections.find((s) => s.id === section)!;
  const writable = project?.role === "owner" || project?.role === "editor";
  return (
    <div className="console-shell">
      <aside className={`sidebar ${mobile ? "is-open" : ""}`}>
        <Link href="/projects" aria-label="GeoLedger 项目">
          <Brand />
        </Link>
        <Button
          className="mobile-close"
          variant="ghost"
          size="icon"
          onClick={() => setMobile(false)}
          aria-label="关闭导航"
        >
          <X />
        </Button>
        <div className="nav-label">WORKSPACE</div>
        <Link href="/projects" className="project-switch">
          <span className="project-avatar">
            {project?.name.slice(0, 1).toUpperCase() ?? "G"}
          </span>
          <span>
            <strong>{project?.name ?? "所有项目"}</strong>
            <small>{project ? role(project.role) : "选择一个项目开始"}</small>
          </span>
          <ChevronRight size={15} />
        </Link>
        <nav aria-label="主导航">
          {sections.map((item, i) => (
            <div key={item.id}>
              {i === 4 && (
                <div className="nav-label nav-divider">ADMINISTRATION</div>
              )}
              <Link
                className={section === item.id ? "active" : ""}
                aria-current={section === item.id ? "page" : undefined}
                href={`/${item.id}${projectId ? `?project=${encodeURIComponent(projectId)}` : ""}`}
              >
                <item.icon size={18} />
                {item.title}
                {section === item.id && <span className="nav-active-dot" />}
              </Link>
            </div>
          ))}
        </nav>
        <div className="sidebar-bottom">
          <div className="service-chip">
            <span className={info ? "status-dot" : "status-dot offline"} />
            <span>
              {info ? `${info.backend.toUpperCase()} · 已连接` : "正在连接服务"}
            </span>
          </div>
          <div className="sidebar-account">
            <span className="account-avatar">
              <Shield size={17} />
            </span>
            <div>
              <strong>当前会话</strong>
              <small>GeoLedger Console</small>
            </div>
            <Button
              variant="ghost"
              size="icon"
              aria-label="退出登录"
              disabled={logout.busy}
              onClick={() =>
                void logout.run(async () => {
                  await send("/api/session", "DELETE");
                  sessionStorage.removeItem("gl.publication");
                  router.replace("/login");
                  router.refresh();
                })
              }
            >
              <LogOut size={17} />
            </Button>
          </div>
          <ErrorBox message={logout.error} />
        </div>
      </aside>
      {mobile && (
        <button
          className="nav-backdrop"
          onClick={() => setMobile(false)}
          aria-label="关闭导航"
        />
      )}
      <div className="main-column">
        <header className="topbar">
          <Button
            variant="ghost"
            size="icon"
            className="mobile-menu"
            aria-label="打开导航"
            onClick={() => setMobile(true)}
          >
            <Menu />
          </Button>
          <span>控制台</span>
          <ChevronRight size={13} />
          <span>{meta.title}</span>
          <div className="topbar-end">
            <Badge variant="outline">
              {info ? `v${info.version}` : "CONSOLE"}
            </Badge>
          </div>
        </header>
        <main className="main-content">
          <div className="page-heading">
            <h1>{meta.title}</h1>
          </div>
          <ErrorBox message={error} />
          {section === "projects" ? (
            <Projects />
          ) : section === "service" ? (
            <ServiceInfo info={info} />
          ) : !projectId ? (
            <Panel>
              <Empty title="先选择一个项目">
                <Link href="/projects">
                  前往项目列表 <ArrowRight className="inline size-4" />
                </Link>
              </Empty>
            </Panel>
          ) : !project ? (
            !error && <Loading />
          ) : (
            <>
              <div className="context-line">
                <Folder size={16} />
                <strong>{project.name}</strong>
                <span className="mono">{short(project.id)}</span>
                <Badge variant="secondary">{role(project.role)}</Badge>
                <span className="context-revision">
                  最新版本 r{project.head}
                </span>
              </div>
              {section === "datasets" && (
                <Datasets project={project} writable={writable} />
              )}
              {section === "workspaces" && (
                <Workspaces project={project} writable={writable} />
              )}
              {section === "history" && (
                <HistoryPanel project={project} writable={writable} />
              )}
              {section === "access" && <Access project={project} />}
              {section === "audit" && <AuditPanel project={project} />}
            </>
          )}
        </main>
        <Sunset />
      </div>
    </div>
  );
}

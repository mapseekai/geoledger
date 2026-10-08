"use client";
import { call, send, type Info, type Project } from "@/lib/browser-api";
import {
  ArrowRight,
  ChevronRight,
  ChevronsUpDown,
  Database,
  FolderOpen,
  GitBranch,
  History,
  LayoutGrid,
  LogOut,
  Menu,
  Server,
  Shield,
  Terminal,
  UserRound,
  X,
  type LucideIcon,
} from "lucide-react";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { useEffect, useState } from "react";
import { Access, AuditPanel } from "./access-audit";
import { Brand } from "./brand";
import { MapConfigProvider } from "./map-config";
import type { RasterBasemap } from "@/lib/basemap";
import { Empty, ErrorBox, Loading, Tip, useMedia } from "./common";
import { HistoryPanel } from "./history";
import { Datasets, Projects, ServiceInfo } from "./projects";
import { StatusBadge, role, short, useAction } from "./resource-shared";
import { Badge } from "./ui/badge";
import { Button } from "./ui/button";
import { Sheet, SheetClose, SheetContent, SheetTitle } from "./ui/sheet";
import { Workspaces } from "./workspaces";
type Section = {
  id: string;
  title: string;
  icon: LucideIcon;
  scoped: boolean;
};
const sections: Section[] = [
  {
    id: "projects",
    title: "项目",
    icon: LayoutGrid,
    scoped: false,
  },
  {
    id: "datasets",
    title: "数据集",
    icon: Database,
    scoped: true,
  },
  {
    id: "workspaces",
    title: "工作区",
    icon: GitBranch,
    scoped: true,
  },
  {
    id: "history",
    title: "版本历史",
    icon: History,
    scoped: true,
  },
  {
    id: "access",
    title: "访问权限",
    icon: Shield,
    scoped: true,
  },
  {
    id: "audit",
    title: "审计日志",
    icon: Terminal,
    scoped: true,
  },
  {
    id: "service",
    title: "服务信息",
    icon: Server,
    scoped: false,
  },
];
const groups = [
  { label: "数据管理", ids: ["projects", "datasets", "workspaces", "history"] },
  { label: "系统管理", ids: ["access", "audit", "service"] },
];
export function Console({
  section,
  projectId,
  basemap,
}: {
  section: string;
  projectId: string;
  basemap?: RasterBasemap;
}) {
  const router = useRouter();
  const [info, setInfo] = useState<Info>(),
    [project, setProject] = useState<Project>(),
    [error, setError] = useState("");
  const [mobile, setMobile] = useState(false);
  const narrow = useMedia("(max-width: 1024px)");
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
  const query = projectId ? `?project=${encodeURIComponent(projectId)}` : "";
  const status = info ? "online" : error ? "offline" : "pending";
  const statusText = info
    ? `${info.backend.toUpperCase()} · 已连接`
    : error
      ? "服务连接失败"
      : "正在连接服务";
  const sidebar = (sheet: boolean) => (
    <>
      <div className="sidebar-header">
        <Link
          href="/projects"
          aria-label="GeoLedger 项目"
          className="sidebar-brand"
        >
          <Brand caption="管理控制台" />
        </Link>
        {sheet && (
          <SheetClose asChild>
            <Button
              className="mobile-close"
              variant="ghost"
              size="icon-sm"
              aria-label="关闭导航"
            >
              <X />
            </Button>
          </SheetClose>
        )}
      </div>
      <div className="sidebar-body">
        <div className="sidebar-label">当前项目</div>
        <Tip label="切换项目" side="right">
          <Link
            href="/projects"
            className="project-switch"
            onClick={() => setMobile(false)}
          >
            <span className="project-avatar" aria-hidden="true">
              {project?.name.slice(0, 1).toUpperCase() ?? (
                <LayoutGrid size={15} />
              )}
            </span>
            <span className="project-switch-text">
              <strong>{project?.name ?? "所有项目"}</strong>
              {project && (
                <small>{`${role(project.role)} · r${project.head}`}</small>
              )}
            </span>
            <ChevronsUpDown size={15} aria-hidden="true" />
          </Link>
        </Tip>
        <nav aria-label="主导航" className="sidebar-nav">
          {groups.map((group) => (
            <div className="nav-group" key={group.label}>
              <div className="sidebar-label">{group.label}</div>
              {group.ids.map((id) => {
                const item = sections.find((s) => s.id === id)!;
                const active = section === item.id;
                return (
                  <Link
                    key={item.id}
                    className={`nav-item ${active ? "active" : ""} ${item.scoped && !projectId ? "is-idle" : ""}`}
                    aria-current={active ? "page" : undefined}
                    href={`/${item.id}${query}`}
                    onClick={() => setMobile(false)}
                  >
                    <item.icon size={17} aria-hidden="true" />
                    <span>{item.title}</span>
                  </Link>
                );
              })}
            </div>
          ))}
        </nav>
      </div>
      <div className="sidebar-footer">
        <div className="service-card">
          <span className={`status-dot is-${status}`} aria-hidden="true" />
          <span className="service-card-text">
            <strong>{statusText}</strong>
            <small>
              {info ? `GeoLedger v${info.version}` : "GeoLedger 服务"}
            </small>
          </span>
        </div>
        <div className="sidebar-account">
          <span className="account-avatar" aria-hidden="true">
            <UserRound size={16} />
          </span>
          <div>
            <strong>当前会话</strong>
          </div>
          <Tip label="退出登录" side="top">
            <Button
              variant="ghost"
              size="icon-sm"
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
              <LogOut />
            </Button>
          </Tip>
        </div>
        <ErrorBox message={logout.error} />
      </div>
    </>
  );
  return (
    <div className="app-shell">
      <aside className="sidebar" aria-label="侧边导航">
        {sidebar(false)}
      </aside>
      <Sheet open={narrow && mobile} onOpenChange={setMobile}>
        <SheetContent
          side="left"
          className="sidebar is-sheet"
          showCloseButton={false}
          aria-describedby={undefined}
        >
          <SheetTitle className="sr-only">侧边导航</SheetTitle>
          {sidebar(true)}
        </SheetContent>
      </Sheet>
      <div className="main-column">
        <header className="topbar">
          <Button
            variant="ghost"
            size="icon-sm"
            className="mobile-menu"
            aria-label="打开导航"
            onClick={() => setMobile(true)}
          >
            <Menu />
          </Button>
          <nav aria-label="面包屑" className="breadcrumbs">
            <Link href="/projects">控制台</Link>
            {project && meta.scoped && (
              <>
                <ChevronRight size={14} aria-hidden="true" />
                <span className="breadcrumb-project" title={project.name}>
                  {project.name}
                </span>
              </>
            )}
            <ChevronRight size={14} aria-hidden="true" />
            <span aria-current="page">{meta.title}</span>
          </nav>
          <div className="topbar-end">
            <Badge variant="outline" className={`status-pill is-${status}`}>
              <span className={`status-dot is-${status}`} aria-hidden="true" />
              {statusText}
            </Badge>
            <Badge variant="secondary" className="version-tag">
              {info ? `v${info.version}` : "CONSOLE"}
            </Badge>
          </div>
        </header>
        <main className="main-content">
          <div className="page-header">
            <span className="page-header-icon" aria-hidden="true">
              <meta.icon />
            </span>
            <h1>{meta.title}</h1>
          </div>
          <ErrorBox message={error} />
          {section === "projects" ? (
            <Projects info={info} />
          ) : section === "service" ? (
            !error && <ServiceInfo info={info} />
          ) : !projectId ? (
            <section className="panel">
              <Empty icon={FolderOpen} title="先选择一个项目">
                <Link href="/projects">
                  前往项目列表 <ArrowRight className="inline size-4" />
                </Link>
              </Empty>
            </section>
          ) : !project ? (
            !error && <Loading />
          ) : (
            <>
              <ProjectContext project={project} />
              {section === "datasets" && (
                <MapConfigProvider value={basemap}>
                  <Datasets project={project} writable={writable} />
                </MapConfigProvider>
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
      </div>
    </div>
  );
}
function ProjectContext({ project }: { project: Project }) {
  return (
    <section className="context-card" aria-label="当前项目">
      <div className="context-main">
        <span className="context-avatar" aria-hidden="true">
          {project.name.slice(0, 1).toUpperCase()}
        </span>
        <div className="context-title">
          <strong>{project.name}</strong>
          <span className="mono" title={project.id}>
            {short(project.id)}
          </span>
        </div>
      </div>
      <dl className="context-stats">
        <div>
          <dt>最新版本</dt>
          <dd className="mono">r{project.head}</dd>
        </div>
        <div>
          <dt>我的角色</dt>
          <dd>
            <StatusBadge value={project.role} />
          </dd>
        </div>
      </dl>
    </section>
  );
}

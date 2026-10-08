"use client";
import { call, type Dataset, type Info, type Project } from "@/lib/browser-api";
import {
  ArrowRight,
  Database,
  Folder,
  Plus,
  Search,
  Server,
} from "lucide-react";
import Link from "next/link";
import { useState } from "react";
import { Empty, ErrorBox, Loading, listPage, usePage } from "./common";
import { FeatureExplorer } from "./features";
import { NameDialog, Panel, short } from "./resource-shared";
import { Badge } from "./ui/badge";
import { Button } from "./ui/button";
import { Input } from "./ui/input";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "./ui/table";
export function Projects() {
  const [refresh, setRefresh] = useState(0),
    [create, setCreate] = useState(false),
    [query, setQuery] = useState("");
  const page = usePage<Project>(
    async (after) =>
      listPage(await call<Project[]>({ action: "projects", after, limit: 20 })),
    refresh,
  );
  const rows = page.rows.filter((p) =>
    p.name.toLowerCase().includes(query.toLowerCase()),
  );
  return (
    <>
      <Panel
        toolbar={
          <>
            <div className="search">
              <Search size={16} />
              <Input
                aria-label="筛选本页项目"
                placeholder="筛选本页项目…"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
              />
            </div>
            <Button onClick={() => setCreate(true)}>
              <Plus />
              创建项目
            </Button>
          </>
        }
      >
        <ErrorBox message={page.error} />
        {page.busy ? (
          <Loading />
        ) : rows.length ? (
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>项目名称</TableHead>
                <TableHead>最新版本</TableHead>
                <TableHead>项目标识</TableHead>
                <TableHead>
                  <span className="sr-only">操作</span>
                </TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {rows.map((p) => (
                <TableRow key={p.id}>
                  <TableCell>
                    <Link
                      className="name-link"
                      href={`/datasets?project=${p.id}`}
                    >
                      <span className="row-icon">
                        <Folder size={18} />
                      </span>
                      {p.name}
                    </Link>
                  </TableCell>
                  <TableCell>
                    <Badge variant="secondary">r{p.head}</Badge>
                  </TableCell>
                  <TableCell className="mono muted">{short(p.id)}</TableCell>
                  <TableCell>
                    <Button asChild variant="ghost" size="icon">
                      <Link
                        aria-label={`打开 ${p.name}`}
                        href={`/datasets?project=${p.id}`}
                      >
                        <ArrowRight />
                      </Link>
                    </Button>
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        ) : (
          <Empty title={query ? "本页没有匹配的项目" : "创建你的第一个项目"} />
        )}{" "}
        {page.footer}
      </Panel>
      {create && (
        <NameDialog
          title="创建项目"
          close={() => setCreate(false)}
          submit={async (name) => {
            await call({ action: "createProject", name });
            setCreate(false);
            setRefresh((n) => n + 1);
            page.reset();
          }}
        />
      )}
    </>
  );
}
export function Datasets({
  project,
  writable,
}: {
  project: Project;
  writable: boolean;
}) {
  const [selected, setSelected] = useState<Dataset>(),
    [create, setCreate] = useState(false),
    [refresh, setRefresh] = useState(0);
  const page = usePage<Dataset>(
    async (after) =>
      listPage(
        await call<Dataset[]>({
          action: "datasets",
          project: project.id,
          after,
          limit: 20,
        }),
      ),
    refresh,
  );
  if (selected)
    return (
      <FeatureExplorer
        key={selected.id}
        project={project}
        dataset={selected}
        writable={writable}
        back={() => setSelected(undefined)}
      />
    );
  return (
    <>
      <Panel
        toolbar={
          <>
            <div>
              <strong>项目数据集</strong>
            </div>
            <Button disabled={!writable} onClick={() => setCreate(true)}>
              <Plus />
              创建数据集
            </Button>
          </>
        }
      >
        <ErrorBox message={page.error} />
        {page.busy ? (
          <Loading />
        ) : page.rows.length ? (
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>数据集名称</TableHead>
                <TableHead>数据集标识</TableHead>
                <TableHead>操作</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {page.rows.map((d) => (
                <TableRow key={d.id}>
                  <TableCell>
                    <button
                      className="name-link"
                      onClick={() => setSelected(d)}
                    >
                      <span className="row-icon">
                        <Database size={18} />
                      </span>
                      {d.name}
                    </button>
                  </TableCell>
                  <TableCell className="mono muted">{d.id}</TableCell>
                  <TableCell>
                    <Button variant="ghost" onClick={() => setSelected(d)}>
                      浏览要素
                      <ArrowRight />
                    </Button>
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        ) : (
          <Empty title="还没有数据集" />
        )}
        {page.footer}
      </Panel>
      {create && (
        <NameDialog
          title="创建数据集"
          close={() => setCreate(false)}
          submit={async (name) => {
            await call({ action: "createDataset", project: project.id, name });
            setCreate(false);
            setRefresh((n) => n + 1);
            page.reset();
          }}
        />
      )}
    </>
  );
}
export function ServiceInfo({ info }: { info?: Info }) {
  return info ? (
    <div className="service-grid">
      <Panel>
        <div className="card-heading">
          <Server />
          <h2>连接信息</h2>
        </div>
        <dl className="details">
          {[
            ["服务版本", info.version],
            ["存储后端", info.backend.toUpperCase()],
            ["存储格式", String(info.formatVersion)],
            ["单要素上限", `${info.maxFeatureBytes.toLocaleString()} bytes`],
            ["单次请求上限", `${info.maxRequestBytes.toLocaleString()} bytes`],
          ].map(([k, v]) => (
            <div key={k}>
              <dt>{k}</dt>
              <dd className="mono">{v}</dd>
            </div>
          ))}
        </dl>
      </Panel>
    </div>
  ) : (
    <Loading />
  );
}

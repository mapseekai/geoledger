"use client";
import {
  call,
  type Dataset,
  type GeometryType,
  type Info,
  type Project,
} from "@/lib/browser-api";
import {
  ArrowRight,
  Database,
  Folder,
  FolderKanban,
  FileJson,
  FolderPlus,
  Gauge,
  GitCommitHorizontal,
  HardDrive,
  Layers,
  Plus,
  Search,
  SearchX,
  Server,
  type LucideIcon,
} from "lucide-react";
import Link from "next/link";
import { useState } from "react";
import { Empty, ErrorBox, Loading, listPage, usePage } from "./common";
import { FeatureExplorer } from "./features";
import {
  NameDialog,
  Panel,
  PanelTitle,
  StatusBadge,
  short,
} from "./resource-shared";
import { Badge } from "./ui/badge";
import { Button } from "./ui/button";
import { Input } from "./ui/input";
import { Label } from "./ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "./ui/select";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "./ui/table";
export function Projects({ info }: { info?: Info }) {
  const [refresh, setRefresh] = useState(0),
    [create, setCreate] = useState(false),
    [query, setQuery] = useState("");
  const [manage, setManage] = useState<{ item: Project; remove: boolean }>();
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
      <div className="stat-grid">
        <Stat
          icon={FolderKanban}
          label="本页项目"
          value={page.busy ? "…" : String(page.rows.length)}
        />
        <Stat
          icon={GitCommitHorizontal}
          label="本页版本合计"
          value={
            page.busy
              ? "…"
              : String(
                  page.rows.reduce((sum, p) => sum + BigInt(p.head), BigInt(0)),
                )
          }
        />
        <Stat
          icon={HardDrive}
          label="存储后端"
          value={info ? info.backend.toUpperCase() : "…"}
          hint={info ? `存储格式 ${info.formatVersion}` : undefined}
        />
        <Stat
          icon={Server}
          label="服务版本"
          value={info ? info.version : "…"}
        />
      </div>
      <Panel
        toolbar={
          <>
            <PanelTitle title="项目列表" />
            <div className="toolbar-actions">
              <div className="search">
                <Search size={15} aria-hidden="true" />
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
            </div>
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
                <TableHead>发布状态</TableHead>
                <TableHead>最新版本</TableHead>
                <TableHead>项目标识</TableHead>
                <TableHead className="cell-actions">
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
                      <span className="row-icon" aria-hidden="true">
                        <Folder size={16} />
                      </span>
                      {p.name}
                    </Link>
                  </TableCell>
                  <TableCell>
                    <StatusBadge
                      value={p.head === "0" ? "unpublished" : "published"}
                    />
                  </TableCell>
                  <TableCell>
                    <Badge variant="outline" className="revision-tag">
                      r{p.head}
                    </Badge>
                  </TableCell>
                  <TableCell className="mono muted" title={p.id}>
                    {short(p.id)}
                  </TableCell>
                  <TableCell className="cell-actions">
                    {p.role === "owner" && (
                      <>
                        <Button
                          variant="ghost"
                          size="sm"
                          onClick={() => setManage({ item: p, remove: false })}
                        >
                          重命名
                        </Button>
                        <Button
                          variant="ghost"
                          size="sm"
                          onClick={() => setManage({ item: p, remove: true })}
                        >
                          删除
                        </Button>
                      </>
                    )}
                    <Button asChild variant="ghost" size="sm">
                      <Link
                        aria-label={`打开 ${p.name}`}
                        href={`/datasets?project=${p.id}`}
                      >
                        进入
                        <ArrowRight />
                      </Link>
                    </Button>
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        ) : (
          <Empty
            icon={query ? SearchX : FolderPlus}
            title={query ? "本页没有匹配的项目" : "创建你的第一个项目"}
          />
        )}
        {page.footer}
      </Panel>
      {manage && (
        <NameDialog
          title={manage.remove ? "删除项目" : "修改项目名称"}
          actionLabel={manage.remove ? "删除" : "保存"}
          initialName={manage.remove ? "" : manage.item.name}
          confirmName={manage.remove ? manage.item.name : undefined}
          description={
            manage.remove
              ? `删除「${manage.item.name}」及全部数据集、工作区和版本历史，此操作不可撤销。`
              : undefined
          }
          close={() => setManage(undefined)}
          submit={async (name) => {
            await call(
              manage.remove
                ? {
                    action: "deleteProject",
                    project: manage.item.id,
                    confirmName: name,
                  }
                : { action: "renameProject", project: manage.item.id, name },
            );
            setManage(undefined);
            setRefresh((n) => n + 1);
            page.reset();
          }}
        />
      )}
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
function Stat({
  icon: Icon,
  label,
  value,
  hint,
}: {
  icon: LucideIcon;
  label: string;
  value: string;
  hint?: string;
}) {
  return (
    <div className="stat-card">
      <div className="stat-card-head">
        <span>{label}</span>
        <span className="stat-icon" aria-hidden="true">
          <Icon />
        </span>
      </div>
      <strong className="stat-value">{value}</strong>
      {hint && <small>{hint}</small>}
    </div>
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
  const [geometryType, setGeometryType] = useState<GeometryType>("point");
  const [coordinateDimension, setCoordinateDimension] = useState<2 | 3>(2);
  const [manage, setManage] = useState<{ item: Dataset; remove: boolean }>();
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
            <PanelTitle title="项目数据集" />
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
                <TableHead>几何类型</TableHead>
                <TableHead>数据集标识</TableHead>
                <TableHead className="cell-actions">操作</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {page.rows.map((d) => (
                <TableRow key={d.id}>
                  <TableCell>
                    <Button
                      variant="link"
                      className="name-link"
                      onClick={() => setSelected(d)}
                    >
                      <span className="row-icon is-data" aria-hidden="true">
                        <Database size={16} />
                      </span>
                      {d.name}
                    </Button>
                  </TableCell>
                  <TableCell>
                    {{ point: "点", line: "线", polygon: "面" }[d.geometryType]}{" "}
                    · {d.coordinateDimension === 3 ? "三维" : "二维"}
                  </TableCell>
                  <TableCell className="mono muted">{d.id}</TableCell>
                  <TableCell className="cell-actions">
                    <Button
                      variant="ghost"
                      size="sm"
                      disabled={!writable}
                      onClick={() => setManage({ item: d, remove: false })}
                    >
                      重命名
                    </Button>
                    {project.role === "owner" && (
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => setManage({ item: d, remove: true })}
                      >
                        删除
                      </Button>
                    )}
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={() => setSelected(d)}
                    >
                      浏览要素
                      <ArrowRight />
                    </Button>
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        ) : (
          <Empty icon={Layers} title="还没有数据集" />
        )}
        {page.footer}
      </Panel>
      {manage && (
        <NameDialog
          title={manage.remove ? "删除数据集" : "修改数据集名称"}
          actionLabel={manage.remove ? "删除" : "保存"}
          initialName={manage.remove ? "" : manage.item.name}
          confirmName={manage.remove ? manage.item.name : undefined}
          description={
            manage.remove
              ? `删除「${manage.item.name}」的要素、工作区变更和版本历史；保留其他数据集的记录，清理空记录。此操作不可撤销。`
              : undefined
          }
          close={() => setManage(undefined)}
          submit={async (name) => {
            await call(
              manage.remove
                ? {
                    action: "deleteDataset",
                    project: project.id,
                    dataset: manage.item.id,
                    confirmName: name,
                  }
                : {
                    action: "renameDataset",
                    project: project.id,
                    dataset: manage.item.id,
                    name,
                  },
            );
            setManage(undefined);
            setRefresh((n) => n + 1);
            page.reset();
          }}
        />
      )}
      {create && (
        <NameDialog
          title="创建数据集"
          close={() => setCreate(false)}
          submit={async (name) => {
            await call({
              action: "createDataset",
              project: project.id,
              name,
              geometryType,
              coordinateDimension,
            });
            setCreate(false);
            setRefresh((n) => n + 1);
            page.reset();
          }}
        >
          <Label htmlFor="geometry-type">几何类型</Label>
          <Select
            value={geometryType}
            onValueChange={(value) => setGeometryType(value as GeometryType)}
          >
            <SelectTrigger id="geometry-type" className="w-full">
              <SelectValue />
            </SelectTrigger>
            <SelectContent position="popper">
              <SelectItem value="point">点（Point / MultiPoint）</SelectItem>
              <SelectItem value="line">
                线（LineString / MultiLineString）
              </SelectItem>
              <SelectItem value="polygon">
                面（Polygon / MultiPolygon）
              </SelectItem>
            </SelectContent>
          </Select>
          <Label htmlFor="coordinate-dimension">坐标维度</Label>
          <Select
            value={String(coordinateDimension)}
            onValueChange={(value) =>
              setCoordinateDimension(Number(value) as 2 | 3)
            }
          >
            <SelectTrigger id="coordinate-dimension" className="w-full">
              <SelectValue />
            </SelectTrigger>
            <SelectContent position="popper">
              <SelectItem value="2">二维（XY）</SelectItem>
              <SelectItem value="3">三维（XYZ）</SelectItem>
            </SelectContent>
          </Select>
          <p className="muted">
            创建后不可更改类型和维度；后续新增和修改必须与数据集一致。
          </p>
        </NameDialog>
      )}
    </>
  );
}
const bytes = (n: number) =>
  n >= 1024 * 1024
    ? `${(n / 1024 / 1024).toLocaleString("zh-CN", { maximumFractionDigits: 1 })} MiB`
    : `${(n / 1024).toLocaleString("zh-CN", { maximumFractionDigits: 1 })} KiB`;
export function ServiceInfo({ info }: { info?: Info }) {
  if (!info) return <Loading rows={3} />;
  return (
    <div className="stat-grid">
      <Stat icon={Server} label="服务版本" value={info.version} />
      <Stat
        icon={HardDrive}
        label="存储后端"
        value={info.backend.toUpperCase()}
        hint={`存储格式 ${info.formatVersion}`}
      />
      <Stat
        icon={FileJson}
        label="单要素上限"
        value={
          info.maxFeatureBytes === 0 ? "不限制" : bytes(info.maxFeatureBytes)
        }
        hint={
          info.maxFeatureBytes === 0
            ? "不设独立字节上限"
            : `${info.maxFeatureBytes.toLocaleString()} bytes`
        }
      />
      <Stat
        icon={Gauge}
        label="单次请求上限"
        value={
          info.maxRequestBytes === 0 ? "不限制" : bytes(info.maxRequestBytes)
        }
        hint={
          info.maxRequestBytes === 0
            ? "支持大数据传输"
            : `${info.maxRequestBytes.toLocaleString()} bytes`
        }
      />
    </div>
  );
}

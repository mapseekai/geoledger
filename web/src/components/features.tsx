"use client";
import {
  ApiError,
  call,
  type Dataset,
  type Feature,
  type FeaturePage,
  type Project,
  type Workspace,
} from "@/lib/browser-api";
import { featureText, pretty } from "@/lib/geojson";
import {
  publication,
  readPublication,
  releasePublication,
  type Publication,
} from "@/lib/publication";
import { ArrowRight, Check, Plus } from "lucide-react";
import { useEffect, useState } from "react";
import { Empty, ErrorBox, Loading, Modal, usePage } from "./common";
import { Panel, short, useAction } from "./resource-shared";
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
import { Textarea } from "./ui/textarea";
export function FeatureExplorer({
  project,
  dataset,
  writable,
  back,
}: {
  project: Project;
  dataset: Dataset;
  writable: boolean;
  back: () => void;
}) {
  const [workspaces, setWorkspaces] = useState<Workspace[]>([]),
    [workspace, setWorkspace] = useState(""),
    [refresh, setRefresh] = useState(0);
  const [workspaceError, setWorkspaceError] = useState("");
  const task = useAction();
  useEffect(() => {
    let active = true;
    call<Workspace[]>({ action: "workspaces", project: project.id, limit: 100 })
      .then((rows) => {
        if (active)
          setWorkspaces((previous) =>
            rows.some((w) => w.id === workspace)
              ? rows
              : [...previous.filter((w) => w.id === workspace), ...rows],
          );
      })
      .catch((e) => {
        if (active) setWorkspaceError(e.message);
      });
    return () => {
      active = false;
    };
  }, [project.id, refresh, workspace]);
  return (
    <>
      <div className="feature-heading">
        <Button variant="outline" onClick={back}>
          返回数据集
        </Button>
        <h2>{dataset.name}</h2>
        <div className="feature-source">
          <label htmlFor="workspace-select">查看</label>
          <select
            id="workspace-select"
            value={workspace}
            onChange={(e) => setWorkspace(e.target.value)}
          >
            <option value="">已发布的数据</option>
            {workspaces
              .filter((w) => w.status === "open")
              .map((w) => (
                <option key={w.id} value={w.id}>
                  工作区 {short(w.id)} · v{w.version}
                </option>
              ))}
          </select>
          <Button
            variant="outline"
            disabled={!writable || task.busy}
            onClick={() =>
              void task.run(async () => {
                const w = await call<Workspace>({
                  action: "createWorkspace",
                  project: project.id,
                });
                setWorkspaces((rows) => [w, ...rows]);
                setWorkspace(w.id);
                setRefresh((n) => n + 1);
              })
            }
          >
            <Plus />
            新建工作区
          </Button>
        </div>
      </div>
      <ErrorBox message={task.error || workspaceError} />
      <details className="workspace-lookup">
        <summary>选择其他工作区</summary>
        <form
          className="lookup"
          onSubmit={(e) => {
            e.preventDefault();
            const id = String(
              new FormData(e.currentTarget).get("workspace"),
            ).trim();
            void task.run(async () => {
              const w = await call<Workspace>({
                action: "workspace",
                project: project.id,
                workspace: id,
              });
              setWorkspaces((rows) => [
                w,
                ...rows.filter((x) => x.id !== w.id),
              ]);
              setWorkspace(w.id);
            });
          }}
        >
          <Input
            name="workspace"
            aria-label="指定工作区标识"
            required
            placeholder="输入工作区标识"
          />
          <Button variant="outline" disabled={task.busy}>
            使用工作区
          </Button>
        </form>
        <p className="subtle-note">
          下拉列表显示前 100 个工作区，也可在这里输入完整标识。
        </p>
      </details>
      <FeatureList
        key={`${dataset.id}:${workspace}`}
        project={project}
        dataset={dataset}
        workspaceId={workspace}
        writable={writable}
      />
    </>
  );
}
export function FeatureList({
  project,
  dataset,
  workspaceId,
  writable,
}: {
  project: Project;
  dataset: Dataset;
  workspaceId: string;
  writable: boolean;
}) {
  const [refresh, setRefresh] = useState(0),
    [version, setVersion] = useState<string>(),
    [inspect, setInspect] = useState<Feature>(),
    [edit, setEdit] = useState<Feature | "new">(),
    [remove, setRemove] = useState<Feature>(),
    [publish, setPublish] = useState(false);
  const [pending, setPending] = useState<Publication>();
  const [snapshot, setSnapshot] = useState<string>(),
    [status, setStatus] = useState("open");
  const task = useAction();
  useEffect(() => {
    setPending(readPublication(sessionStorage.getItem("gl.publication")));
  }, []);
  const page = usePage<Feature>(async (after) => {
    const result = await call<FeaturePage>({
      action: "features",
      project: project.id,
      dataset: dataset.id,
      workspace: workspaceId || undefined,
      revision: !workspaceId && after ? snapshot : undefined,
      after,
      limit: 20,
    });
    if (workspaceId) {
      const w = await call<Workspace>({
        action: "workspace",
        project: project.id,
        workspace: workspaceId,
      });
      if (result.workspaceVersion !== w.version)
        throw new Error("工作区已变化。请刷新列表，从第一页重新查看。");
      return {
        rows: result.features,
        next: result.nextAfter,
        identity: result.workspaceVersion,
        accept: () => {
          setStatus(w.status);
          setVersion(result.workspaceVersion);
        },
      };
    }
    return {
      rows: result.features,
      next: result.nextAfter,
      accept: () => setSnapshot(result.revision),
    };
  }, refresh);
  const canEdit =
    writable &&
    !!workspaceId &&
    status === "open" &&
    version !== undefined &&
    !pending &&
    !page.busy &&
    !page.error;
  return (
    <>
      <Panel
        toolbar={
          <>
            <div>
              <strong>{workspaceId ? "工作区要素" : "已发布要素"}</strong>
              <Badge variant="secondary">
                {workspaceId ? `v${version ?? "…"}` : `r${snapshot ?? "…"}`}
              </Badge>
            </div>
            <div className="toolbar-actions">
              <Button
                variant="outline"
                disabled={!canEdit}
                onClick={() => setEdit("new")}
              >
                <Plus />
                添加要素
              </Button>
              {workspaceId && (
                <Button
                  disabled={
                    !writable ||
                    (status !== "open" && !pending) ||
                    version === undefined ||
                    page.busy
                  }
                  onClick={() => setPublish(true)}
                >
                  发布版本
                  <ArrowRight />
                </Button>
              )}
            </div>
          </>
        }
      >
        <ErrorBox message={page.error || task.error} />
        {pending && (
          <div className="notice">
            有一笔待确认的发布请求。请到对应工作区重试原请求，再继续编辑。
          </div>
        )}
        {page.busy ? (
          <Loading />
        ) : page.rows.length ? (
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>要素标识</TableHead>
                <TableHead>内容</TableHead>
                <TableHead>操作</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {page.rows.map((f) => (
                <TableRow key={f.id}>
                  <TableCell className="mono">{f.id}</TableCell>
                  <TableCell>
                    <button className="text-link" onClick={() => setInspect(f)}>
                      查看 GeoJSON
                    </button>
                  </TableCell>
                  <TableCell>
                    <div className="toolbar-actions">
                      <Button
                        variant="ghost"
                        disabled={!canEdit}
                        onClick={() => setEdit(f)}
                      >
                        编辑
                      </Button>
                      <Button
                        variant="ghost"
                        disabled={!canEdit}
                        onClick={() => setRemove(f)}
                      >
                        删除
                      </Button>
                    </div>
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        ) : (
          <Empty title="这个视图下还没有要素">
            {workspaceId
              ? "添加要素后，发布工作区以生成新版本。"
              : "新建工作区，在工作区中添加并发布要素。"}
          </Empty>
        )}
        {page.footer}
      </Panel>
      {inspect && (
        <Modal
          title={`要素 ${inspect.id}`}
          description="GeoJSON"
          close={() => setInspect(undefined)}
        >
          <pre className="json-view">{pretty(inspect.geojson)}</pre>
        </Modal>
      )}
      {edit && (
        <FeatureEditor
          feature={edit}
          close={() => setEdit(undefined)}
          save={async (id, raw) => {
            await call({
              action: "save",
              project: project.id,
              workspace: workspaceId,
              version,
              edits: [{ dataset: dataset.id, featureId: id, feature: raw }],
            });
            setEdit(undefined);
            setRefresh((n) => n + 1);
            page.reset();
          }}
        />
      )}
      {remove && (
        <Modal
          title="删除要素"
          description={`将在当前工作区中删除 ${remove.id}。发布前不会改变正式版本。`}
          close={task.busy ? () => {} : () => setRemove(undefined)}
        >
          <ErrorBox message={task.error} />
          <div className="form-actions">
            <Button
              variant="outline"
              onClick={() => setRemove(undefined)}
              disabled={task.busy}
            >
              取消
            </Button>
            <Button
              disabled={task.busy}
              onClick={() =>
                void task.run(async () => {
                  await call({
                    action: "save",
                    project: project.id,
                    workspace: workspaceId,
                    version,
                    edits: [
                      {
                        dataset: dataset.id,
                        featureId: remove.id,
                        feature: null,
                      },
                    ],
                  });
                  setRemove(undefined);
                  setRefresh((n) => n + 1);
                  page.reset();
                })
              }
            >
              确认删除
            </Button>
          </div>
        </Modal>
      )}
      {publish && (
        <PublishDialog
          project={project.id}
          workspace={workspaceId}
          version={version!}
          close={() => {
            setPublish(false);
            setPending(
              readPublication(sessionStorage.getItem("gl.publication")),
            );
          }}
          complete={() => {
            setPending(undefined);
            setRefresh((n) => n + 1);
            page.reset();
          }}
        />
      )}
    </>
  );
}
export function FeatureEditor({
  feature,
  close,
  save,
}: {
  feature: Feature | "new";
  close: () => void;
  save: (id: string, raw: string) => Promise<void>;
}) {
  const task = useAction();
  return (
    <Modal
      title={feature === "new" ? "添加要素" : "编辑要素"}
      description="输入 GeoJSON Feature，发布后生效。"
      close={task.busy ? () => {} : close}
    >
      <form
        onSubmit={(e) => {
          e.preventDefault();
          const form = new FormData(e.currentTarget);
          const id =
            feature === "new" ? String(form.get("id")).trim() : feature.id;
          void task.run(() =>
            save(id, featureText(String(form.get("geojson")), id)),
          );
        }}
      >
        <label htmlFor="feature-id">要素标识</label>
        <Input
          id="feature-id"
          name="id"
          required
          maxLength={256}
          defaultValue={feature === "new" ? "" : feature.id}
          readOnly={feature !== "new"}
        />
        <label htmlFor="geojson">GeoJSON</label>
        <Textarea
          className="code-editor"
          id="geojson"
          name="geojson"
          required
          rows={14}
          spellCheck={false}
          defaultValue={
            feature === "new"
              ? '{\n  "type": "Feature",\n  "properties": {},\n  "geometry": {\n    "type": "Point",\n    "coordinates": [104, 35]\n  }\n}'
              : pretty(feature.geojson)
          }
        />
        <ErrorBox message={task.error} />
        <div className="form-actions">
          <Button
            type="button"
            variant="outline"
            onClick={close}
            disabled={task.busy}
          >
            取消
          </Button>
          <Button type="submit" disabled={task.busy}>
            {task.busy ? "正在保存…" : "保存到工作区"}
          </Button>
        </div>
      </form>
    </Modal>
  );
}
export function PublishDialog({
  project,
  workspace,
  version,
  close,
  complete,
}: {
  project: string;
  workspace: string;
  version: string;
  close: () => void;
  complete: () => void;
}) {
  const [pending, setPending] = useState(() =>
    readPublication(sessionStorage.getItem("gl.publication")),
  );
  const [message, setMessage] = useState(pending?.message ?? "");
  const [result, setResult] = useState<string>();
  const task = useAction();
  const matches =
    !pending ||
    (pending.project === project && pending.workspace === workspace);
  return (
    <Modal
      title={result ? "发布完成" : "发布新版本"}
      description="合并工作区更改；有冲突时需先解决。"
      close={task.busy ? () => {} : close}
    >
      {result ? (
        <div className="publish-success">
          <Check />
          <h3>版本 r{result} 已发布</h3>
          <Button onClick={close}>完成</Button>
        </div>
      ) : (
        <form
          onSubmit={(e) => {
            e.preventDefault();
            void task.run(async () => {
              const intent =
                pending ??
                publication(project, workspace, version, message.trim());
              // Persist before sending: an interrupted response can only retry this immutable request.
              sessionStorage.setItem("gl.publication", JSON.stringify(intent));
              setPending(intent);
              try {
                const response = await call<{ revision: string }>(intent);
                sessionStorage.removeItem("gl.publication");
                setPending(undefined);
                setResult(response.revision);
                complete();
              } catch (error) {
                if (
                  error instanceof ApiError &&
                  releasePublication(error.status, error.uncertain, !!pending)
                ) {
                  sessionStorage.removeItem("gl.publication");
                  setPending(undefined);
                }
                throw error;
              }
            });
          }}
        >
          <label htmlFor="publish-message">版本说明</label>
          <Textarea
            id="publish-message"
            required
            maxLength={2048}
            value={message}
            readOnly={!!pending}
            onChange={(e) => setMessage(e.target.value)}
            placeholder="例如：更新道路边界与分类"
          />
          {pending && (
            <div className="notice">
              {matches
                ? "保留了原始发布请求，重试会使用相同内容和请求标识。"
                : `请先到工作区 ${pending.workspace} 确认上一笔发布。`}
            </div>
          )}
          <ErrorBox message={task.error} />
          <div className="form-actions">
            <Button
              type="button"
              variant="outline"
              disabled={task.busy}
              onClick={close}
            >
              关闭
            </Button>
            <Button
              type="submit"
              disabled={task.busy || !matches || !message.trim()}
            >
              {task.busy ? "正在发布…" : pending ? "重试原发布" : "确认发布"}
            </Button>
          </div>
        </form>
      )}
    </Modal>
  );
}

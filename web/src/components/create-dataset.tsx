"use client";
import { useRef, useState } from "react";
import {
  call,
  type Dataset,
  type GeometryType,
  type Workspace,
} from "@/lib/browser-api";
import { importBatch, type ImportedFeature } from "@/lib/geojson-import";
import { recoverImportBatch } from "@/lib/import-recovery";
import { ErrorBox, Modal } from "./common";
import { useAction } from "./resource-shared";
import { Button } from "./ui/button";
import { Input } from "./ui/input";
import { Field, FieldGroup, FieldLabel } from "./ui/field";
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "./ui/select";

export function CreateDatasetDialog({
  project,
  backend,
  close,
  created,
}: {
  project: string;
  backend?: string;
  close: () => void;
  created: (dataset: Dataset, workspace?: string) => void;
}) {
  const task = useAction();
  const [mode, setMode] = useState("empty");
  const [family, setFamily] = useState<GeometryType>("point");
  const [dimension, setDimension] = useState<2 | 3>(2);
  const [progress, setProgress] = useState("");
  const state = useRef<{
    dataset?: Dataset;
    workspace?: Workspace;
    rows?: ImportedFeature[];
    offset: number;
    pending?: boolean;
  }>({ offset: 0 });
  const [started, setStarted] = useState(false);
  return (
    <Modal
      title="创建数据集"
      description="从 GeoJSON 文件创建数据集、接入已有 PostGIS 表，或创建空数据集。"
      close={task.busy ? () => {} : close}
    >
      <form
        onSubmit={(e) => {
          e.preventDefault();
          const form = new FormData(e.currentTarget);
          const name = String(form.get("name")).trim();
          void task.run(async () => {
            const current = state.current;
            if (mode === "postgis") {
              const dataset = await call<Dataset>({
                action: "attachPostgisTable",
                project,
                name,
                source: {
                  schema: form.get("schema"),
                  table: form.get("table"),
                  idColumn: form.get("idColumn"),
                  geometryColumn: form.get("geometryColumn"),
                },
              });
              created(dataset);
              return;
            }
            let shape = {
              geometryType: family,
              coordinateDimension: dimension,
            };
            if (mode === "file" && !current.rows) {
              const file = form.get("file") as File;
              if (!file?.size)
                throw new Error("请选择包含要素的 GeoJSON 文件。");
              setProgress("正在读取文件并识别几何类型与坐标维度…");
              const result = await new Promise<{
                rows: ImportedFeature[];
                shape?: typeof shape;
              }>((resolve, reject) => {
                const worker = new Worker(
                  new URL("../lib/geojson-import.worker.ts", import.meta.url),
                );
                worker.onmessage = (event) => {
                  worker.terminate();
                  event.data.error
                    ? reject(new Error(event.data.error))
                    : resolve(event.data);
                };
                worker.onerror = () => {
                  worker.terminate();
                  reject(new Error("GeoJSON 解析失败，请重试。"));
                };
                worker.postMessage({ file });
              });
              current.rows = result.rows;
              shape = result.shape ?? shape;
              setFamily(shape.geometryType);
              setDimension(shape.coordinateDimension);
            }
            if (!current.dataset) {
              current.dataset = await call<Dataset>({
                action: "createDataset",
                project,
                name,
                ...shape,
              });
              setStarted(true);
            }
            if (current.rows) {
              current.workspace ??= await call<Workspace>({
                action: "createWorkspace",
                project,
              });
              while (current.offset < current.rows.length) {
                const batch = importBatch(current.rows, current.offset);
                const recovered: Workspace | undefined = current.pending
                  ? await recoverImportBatch(
                      project,
                      current.dataset.id,
                      current.workspace,
                      batch,
                    )
                  : undefined;
                current.pending = true;
                if (recovered) current.workspace.version = recovered.version;
                else {
                  const saved = await call<{ version: string }>({
                    action: "save",
                    project,
                    workspace: current.workspace.id,
                    version: current.workspace.version,
                    edits: batch.map((row) => ({
                      dataset: current.dataset!.id,
                      featureId: row.id,
                      feature: row.raw,
                    })),
                  });
                  current.workspace.version = saved.version;
                }
                current.pending = false;
                current.offset += batch.length;
                setProgress(
                  `已导入 ${current.offset} / ${current.rows.length} 个要素到工作区。`,
                );
              }
            }
            created(current.dataset, current.workspace?.id);
          });
        }}
      >
        <fieldset disabled={task.busy || started}>
          <FieldGroup>
            <Field>
              <FieldLabel htmlFor="dataset-name">名称</FieldLabel>
              <Input
                id="dataset-name"
                name="name"
                required
                maxLength={256}
                autoFocus
              />
            </Field>
            <Field>
              <FieldLabel htmlFor="dataset-source">数据来源</FieldLabel>
              <Select
                value={mode}
                onValueChange={(value) => {
                  setMode(value);
                  state.current = { offset: 0 };
                  setProgress("");
                }}
              >
                <SelectTrigger id="dataset-source">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectGroup>
                    <SelectItem value="file">上传 GeoJSON 文件</SelectItem>
                    {backend === "postgis" && (
                      <SelectItem value="postgis">
                        接入 PostGIS 已有表
                      </SelectItem>
                    )}
                    <SelectItem value="empty">空数据集</SelectItem>
                  </SelectGroup>
                </SelectContent>
              </Select>
            </Field>
            {mode === "postgis" ? (
              <>
                <Field>
                  <FieldLabel htmlFor="source-schema">Schema</FieldLabel>
                  <Input
                    id="source-schema"
                    name="schema"
                    defaultValue="public"
                    required
                    maxLength={63}
                  />
                </Field>
                <Field>
                  <FieldLabel htmlFor="source-table">数据表</FieldLabel>
                  <Input
                    id="source-table"
                    name="table"
                    required
                    maxLength={63}
                  />
                </Field>
                <Field>
                  <FieldLabel htmlFor="source-id">主键列</FieldLabel>
                  <Input
                    id="source-id"
                    name="idColumn"
                    defaultValue="id"
                    required
                    maxLength={63}
                  />
                </Field>
                <Field>
                  <FieldLabel htmlFor="source-geometry">几何列</FieldLabel>
                  <Input
                    id="source-geometry"
                    name="geometryColumn"
                    defaultValue="geom"
                    required
                    maxLength={63}
                  />
                </Field>
                <p>
                  由平台管理员接入当前数据库中的业务表，自动读取类型和维度并建立初始版本。发布时同步修改原表，纳管后的写入通过
                  GeoLedger 完成。
                </p>
              </>
            ) : (
              <>
                {mode === "file" && (
                  <Field>
                    <FieldLabel htmlFor="geojson-file">GeoJSON 文件</FieldLabel>
                    <Input
                      id="geojson-file"
                      name="file"
                      type="file"
                      accept=".json,.geojson,application/json,application/geo+json"
                      required
                      onChange={() => {
                        state.current = { offset: 0 };
                        setProgress("");
                      }}
                    />
                    <p>
                      自动识别文件中的几何类型和坐标维度，创建后将要素保存到新工作区，检查后发布。以下选项用于全部几何为空的文件。
                    </p>
                  </Field>
                )}
                <Field>
                  <FieldLabel htmlFor="geometry-type">几何类型</FieldLabel>
                  <Select
                    value={family}
                    onValueChange={(v) => setFamily(v as GeometryType)}
                  >
                    <SelectTrigger id="geometry-type">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectGroup>
                        <SelectItem value="point">
                          点（Point / MultiPoint）
                        </SelectItem>
                        <SelectItem value="line">
                          线（LineString / MultiLineString）
                        </SelectItem>
                        <SelectItem value="polygon">
                          面（Polygon / MultiPolygon）
                        </SelectItem>
                      </SelectGroup>
                    </SelectContent>
                  </Select>
                </Field>
                <Field>
                  <FieldLabel htmlFor="coordinate-dimension">
                    坐标维度
                  </FieldLabel>
                  <Select
                    value={String(dimension)}
                    onValueChange={(v) => setDimension(Number(v) as 2 | 3)}
                  >
                    <SelectTrigger id="coordinate-dimension">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectGroup>
                        <SelectItem value="2">二维（XY）</SelectItem>
                        <SelectItem value="3">三维（XYZ）</SelectItem>
                      </SelectGroup>
                    </SelectContent>
                  </Select>
                </Field>
              </>
            )}
          </FieldGroup>
        </fieldset>
        {progress && <p role="status">{progress}</p>}
        {started && (
          <p>
            数据集已创建。导入中断时可重试继续，或稍后在工作区检查已保存的要素。
          </p>
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
          <Button type="submit" disabled={task.busy}>
            {task.busy ? "正在处理…" : started ? "继续导入" : "创建"}
          </Button>
        </div>
      </form>
    </Modal>
  );
}

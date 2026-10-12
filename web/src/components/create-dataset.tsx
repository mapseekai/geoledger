"use client";
import { useRef, useState } from "react";
import { Plus, Upload } from "lucide-react";
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
import { Field, FieldDescription, FieldGroup, FieldLabel } from "./ui/field";
import { ToggleGroup, ToggleGroupItem } from "./ui/toggle-group";
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
    shape?: {
      geometryType: GeometryType;
      coordinateDimension: 2 | 3;
      crs: string;
    };
    offset: number;
    pending?: boolean;
  }>({ offset: 0 });
  const [started, setStarted] = useState(false);
  return (
    <Modal title="创建数据集" busy={task.busy} close={close}>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          const form = new FormData(e.currentTarget);
          const name = String(form.get("name")).trim();
          void task.run(async () => {
            const current = state.current;
            if (mode === "postgis") {
              const dataset = await call<Dataset>({
                action: "trackTable",
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
              crs: String(form.get("crs") || "EPSG:4326").trim(),
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
                  if (event.data.error) reject(new Error(event.data.error));
                  else resolve(event.data);
                };
                worker.onerror = () => {
                  worker.terminate();
                  reject(new Error("GeoJSON 解析失败，请重试。"));
                };
                worker.postMessage({
                  file,
                  crs: String(form.get("crs") || "").trim() || undefined,
                });
              });
              if (!result.shape)
                throw new Error(
                  "请上传至少包含一个有坐标要素的文件，以自动识别几何类型和坐标维度。",
                );
              current.rows = result.rows;
              current.shape = result.shape;
            }
            if (mode === "file" && current.shape) shape = current.shape;
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
              <FieldLabel id="dataset-mode-label">创建方式</FieldLabel>
              <ToggleGroup
                type="single"
                variant="outline"
                className="w-full"
                aria-labelledby="dataset-mode-label"
                value={mode === "file" ? "file" : "create"}
                disabled={task.busy || started}
                onValueChange={(value) => {
                  if (!value) return;
                  setMode(value === "file" ? "file" : "empty");
                  state.current = { offset: 0 };
                  setProgress("");
                }}
              >
                <ToggleGroupItem value="create" className="flex-1">
                  <Plus />
                  创建模式
                </ToggleGroupItem>
                <ToggleGroupItem value="file" className="flex-1">
                  <Upload />
                  上传模式
                </ToggleGroupItem>
              </ToggleGroup>
            </Field>
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
            {backend === "postgis" && mode !== "file" && (
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
                      <SelectItem value="postgis">
                        纳管已有 PostGIS 表
                      </SelectItem>
                      <SelectItem value="empty">空数据集</SelectItem>
                    </SelectGroup>
                  </SelectContent>
                </Select>
              </Field>
            )}
            {mode !== "postgis" && (
              <Field>
                <FieldLabel htmlFor="dataset-crs">坐标系</FieldLabel>
                <Input
                  id="dataset-crs"
                  name="crs"
                  maxLength={16384}
                  placeholder="EPSG:4326、EPSG:4490 或 WKT / PROJ 定义"
                />
                <FieldDescription>
                  保留原始坐标。留空时读取文件声明，无声明则使用
                  EPSG:4326；创建后不可更改。
                </FieldDescription>
              </Field>
            )}
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
                    <FieldDescription>
                      根据文件内容自动识别几何类型和坐标维度。
                    </FieldDescription>
                  </Field>
                )}
                {mode === "empty" && (
                  <>
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
              </>
            )}
          </FieldGroup>
        </fieldset>
        {progress && <p role="status">{progress}</p>}
        {started && <p className="muted">数据集已创建</p>}
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
            {task.busy
              ? "正在处理…"
              : started
                ? "继续导入"
                : mode === "postgis"
                  ? "纳管"
                  : "创建"}
          </Button>
        </div>
      </form>
    </Modal>
  );
}

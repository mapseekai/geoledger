"use client";

import {
  ApiError,
  call,
  type Conflicts,
  type Workspace,
} from "@/lib/browser-api";
import {
  fieldLabel,
  isWholeFeatureConflict,
  mergeConflict,
  parseConflict,
  parseLosslessJson,
  propertyForPointer,
  type Conflict,
  type ConflictSide,
} from "@/lib/conflicts";
import { featureText } from "@/lib/geojson";
import { readPublication } from "@/lib/publication";
import { stringify } from "lossless-json";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ErrorBox, Loading, Modal, Notice } from "./common";
import styles from "./conflict-resolution.module.css";
import { Badge } from "./ui/badge";
import { Button } from "./ui/button";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "./ui/collapsible";
import { Textarea } from "./ui/textarea";

type Selection = {
  fields: Partial<Record<string, ConflictSide>>;
  text?: string;
};
type LoadedConflict = Conflict & { cursor: string };
type ResolutionResult = {
  version: string;
  head: string;
  remainingConflicts: string;
};

function sideText(feature: Conflict[ConflictSide]): string {
  return feature ? stringify(feature, undefined, 2)! : "删除要素";
}
function conflictKey(conflict: LoadedConflict): string {
  return `${conflict.dataset}\u0000${conflict.featureId}`;
}
function choiceText(side: ConflictSide): string {
  return side === "current"
    ? "已发布版本"
    : side === "draft"
      ? "我的草稿"
      : "基线";
}

export function Resolution({
  project,
  workspace,
  mode,
  close,
  complete,
  readOnly = false,
}: {
  project: string;
  workspace: Workspace;
  mode: "resolve" | "rebase";
  close: () => void;
  complete: () => void;
  readOnly?: boolean;
}) {
  const [data, setData] = useState<Conflicts>();
  const [items, setItems] = useState<LoadedConflict[]>([]);
  const [cursors, setCursors] = useState([""]);
  const after = cursors.at(-1)!;
  const [selected, setSelected] = useState(0);
  const [choices, setChoices] = useState<Record<string, Selection>>({});
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [reload, setReload] = useState(0);
  const [reconfirm, setReconfirm] = useState(false);
  const [editor, setEditor] = useState("");
  const [customValid, setCustomValid] = useState(true);
  const [customDirty, setCustomDirty] = useState(false);
  const snapshot = useRef<string | undefined>(undefined);

  const load = useCallback(async () => {
    setBusy(true);
    setError("");
    try {
      const result = await call<Conflicts>({
        action: "conflicts",
        project,
        workspace: workspace.id,
        after,
        limit: 20,
      });
      const identity = `${result.version}:${result.head}`;
      if (snapshot.current && snapshot.current !== identity) {
        setChoices({});
        setSelected(0);
        setCursors([""]);
      }
      snapshot.current = identity;
      setData(result);
      setItems(
        result.conflicts.map((entry) => ({
          ...parseConflict(entry.json),
          cursor: entry.cursor,
        })),
      );
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "无法加载冲突。");
    } finally {
      setBusy(false);
    }
  }, [after, project, reload, workspace.id]);

  useEffect(() => {
    void load();
  }, [load]);

  const current = items[Math.min(selected, Math.max(0, items.length - 1))];
  const currentKey = current && conflictKey(current);
  const selection = currentKey
    ? (choices[currentKey] ?? { fields: {} })
    : { fields: {} };
  const automatic = useMemo(
    () => (current ? mergeConflict(current, selection.fields) : undefined),
    [current, selection.fields],
  );
  const wholeSide = selection.fields["*"];
  const resultText =
    selection.text ??
    (wholeSide && !current?.[wholeSide] ? null : automatic?.text);
  const whole = current ? isWholeFeatureConflict(current) : false;

  useEffect(() => {
    setEditor(selection.text ?? resultText ?? "");
    setCustomValid(true);
    setCustomDirty(false);
  }, [currentKey, resultText, selection.text]);

  const choose = (field: string, side: ConflictSide) => {
    if (!currentKey || busy) return;
    setReconfirm(false);
    setCustomDirty(false);
    setChoices((previous) => {
      const prior = previous[currentKey] ?? { fields: {} };
      const { ["*"]: whole, ...fields } = prior.fields;
      return {
        ...previous,
        [currentKey]: {
          fields:
            field === "*"
              ? { ...prior.fields, "*": side }
              : { ...fields, [field]: side },
        },
      };
    });
  };
  const setCustomText = (text: string) => {
    if (!current || busy) return;
    setEditor(text);
    setCustomDirty(true);
    try {
      featureText(text, current.featureId);
      setCustomValid(true);
      setError("");
    } catch (cause) {
      setCustomValid(false);
      setError(cause instanceof Error ? cause.message : "GeoJSON 无效。");
    }
  };
  const applyCustomText = () => {
    if (!currentKey || !current || !customValid || busy) return;
    const normalized = featureText(editor, current.featureId);
    setChoices((previous) => ({
      ...previous,
      [currentKey]: {
        fields: previous[currentKey]?.fields ?? {},
        text: normalized,
      },
    }));
    setCustomDirty(false);
  };
  const edits = items.flatMap((item) => {
    const choice = choices[conflictKey(item)] ?? { fields: {} };
    const wholeSide = choice.fields["*"];
    const result =
      choice.text ??
      (wholeSide && !item[wholeSide]
        ? null
        : mergeConflict(item, choice.fields)?.text);
    return result === undefined
      ? []
      : [{ dataset: item.dataset, featureId: item.featureId, feature: result }];
  });
  const canRebase = data && data.total === "0";

  const submit = async () => {
    if (!data || readOnly) return;
    if (readPublication(sessionStorage.getItem("gl.publication"))) {
      setError("请先确认待处理的发布请求。");
      return;
    }
    if (mode === "resolve" && edits.length === 0) {
      setError("请先选择至少一个冲突结果。");
      return;
    }
    if (mode === "rebase" && !canRebase && edits.length === 0) {
      setError("请先确认本页的冲突结果。");
      return;
    }
    setBusy(true);
    setError("");
    try {
      const result = await call<ResolutionResult>({
        action: mode === "rebase" && canRebase ? "rebase" : "resolve",
        project,
        workspace: workspace.id,
        version: data.version,
        head: data.head,
        edits: mode === "rebase" && canRebase ? [] : edits,
      });
      if (
        (mode === "rebase" && canRebase) ||
        (mode === "resolve" && result.remainingConflicts === "0")
      ) {
        complete();
        return;
      }
      setChoices({});
      setCursors([""]);
      setSelected(0);
      setReload((value) => value + 1);
    } catch (cause) {
      const message = cause instanceof Error ? cause.message : "保存失败。";
      setError(
        cause instanceof ApiError && cause.status === 409
          ? `${message} 已重新加载冲突，请重新确认。`
          : message,
      );
      if (
        cause instanceof ApiError &&
        (cause.status === 409 || cause.uncertain)
      )
        setReconfirm(true);
      if (
        cause instanceof ApiError &&
        (cause.status === 409 || cause.uncertain)
      ) {
        setChoices({});
        setCursors([""]);
        setSelected(0);
        setReload((value) => value + 1);
      }
    } finally {
      setBusy(false);
    }
  };

  return (
    <Modal
      title={
        readOnly
          ? "查看冲突"
          : mode === "resolve"
            ? "解决冲突"
            : "更新工作区基准"
      }
      description="已发布版本与草稿并排显示；仅已确认的结果会保存。"
      close={busy ? () => {} : close}
      className={styles.dialog}
    >
      <ErrorBox message={error} />
      {reconfirm && (
        <Notice tone="warning">
          请求结果可能已改变冲突上下文，已重新加载；请重新确认选择。
        </Notice>
      )}
      {busy && !data ? (
        <Loading />
      ) : !data ? (
        <Notice
          tone="warning"
          action={
            <Button
              variant="outline"
              size="sm"
              onClick={() => setReload((n) => n + 1)}
            >
              重试
            </Button>
          }
        >
          无法确认冲突状态。
        </Notice>
      ) : !current ? (
        <Notice tone="success">当前没有待审阅的冲突。</Notice>
      ) : (
        <div className={styles.layout}>
          <nav className={styles.list} aria-label="冲突要素">
            {items.map((item, index) => {
              const choice = choices[conflictKey(item)] ?? { fields: {} };
              const wholeSide = choice.fields["*"];
              const ready =
                choice.text !== undefined ||
                (wholeSide !== undefined && !item[wholeSide]) ||
                mergeConflict(item, choice.fields) !== undefined;
              return (
                <Button
                  variant={index === selected ? "secondary" : "outline"}
                  aria-current={index === selected ? "true" : undefined}
                  className={
                    index === selected ? styles.activeItem : styles.item
                  }
                  key={item.cursor}
                  onClick={() => {
                    if (!busy) setSelected(index);
                  }}
                  disabled={busy}
                  type="button"
                >
                  <strong>{item.dataset}</strong>
                  <span>{item.featureId}</span>
                  <Badge variant={ready ? "secondary" : "outline"}>
                    {ready ? "已确认" : "待确认"}
                  </Badge>
                </Button>
              );
            })}
            {Object.keys(choices).length > 0 && (
              <p className={styles.pageHint}>请先保存本页确认，再翻页。</p>
            )}
            {data?.truncated && data.nextAfter && (
              <Button
                variant="outline"
                size="sm"
                disabled={busy || Object.keys(choices).length > 0}
                onClick={() => {
                  setCursors((value) => [...value, data.nextAfter!]);
                  setSelected(0);
                }}
              >
                下一页
              </Button>
            )}
            {cursors.length > 1 && (
              <Button
                variant="outline"
                size="sm"
                disabled={busy || Object.keys(choices).length > 0}
                onClick={() => {
                  setCursors((value) => value.slice(0, -1));
                  setSelected(0);
                }}
              >
                上一页
              </Button>
            )}
          </nav>
          <section className={styles.detail} aria-live="polite">
            <div className={styles.heading}>
              <div>
                <strong>{current.dataset}</strong> / {current.featureId}
              </div>
              {current.reason === "stale_resolution" && (
                <Badge variant="destructive">上下文已变化，需重新确认</Badge>
              )}
            </div>
            <ComparisonTable
              conflict={current}
              publishedRevision={data?.head ?? "…"}
            />
            <Collapsible>
              <CollapsibleTrigger asChild>
                <Button variant="ghost" size="sm">
                  查看原始要素 JSON 与基线
                </Button>
              </CollapsibleTrigger>
              <CollapsibleContent>
                <div className={styles.columns}>
                  <FeatureColumn
                    title={`已发布 r${data?.head ?? "…"}`}
                    feature={current.current}
                  />
                  <FeatureColumn title="我的草稿" feature={current.draft} />
                  <FeatureColumn title="基线" feature={current.base} />
                </div>
              </CollapsibleContent>
            </Collapsible>
            {!readOnly && (
              <div className={styles.choices}>
                <h3>选择结果</h3>
                <p>“整个要素”会覆盖该要素的全部字段。</p>
                <div className={styles.wholeChoices}>
                  {(["current", "draft", "base"] as const).map((side) => (
                    <Button
                      key={side}
                      type="button"
                      variant={
                        selection.fields["*"] === side ? "default" : "outline"
                      }
                      disabled={busy}
                      onClick={() => choose("*", side)}
                    >
                      采用{choiceText(side)}整个要素
                    </Button>
                  ))}
                </div>
                {!whole &&
                  current.fields.map((field) => (
                    <div className={styles.field} key={field}>
                      <div>
                        <strong>{fieldLabel(field)}</strong>
                        <span className={styles.fieldValues}>
                          已发布：{fieldValue(current.current, field)} · 草稿：
                          {fieldValue(current.draft, field)}
                        </span>
                      </div>
                      <div>
                        {(["current", "draft", "base"] as const).map((side) => (
                          <Button
                            key={side}
                            type="button"
                            size="sm"
                            variant={
                              selection.fields[field] === side
                                ? "default"
                                : "outline"
                            }
                            disabled={busy}
                            onClick={() => choose(field, side)}
                          >
                            {choiceText(side)}
                          </Button>
                        ))}
                      </div>
                    </div>
                  ))}
              </div>
            )}
            <div className={styles.preview}>
              <h3>结果预览</h3>
              <pre className={styles.code}>
                {resultText === null
                  ? "删除要素"
                  : resultText
                    ? stringify(parseLosslessJson(resultText), undefined, 2)
                    : "请选择一个结果。"}
              </pre>
            </div>
            {!readOnly && (
              <Collapsible>
                <CollapsibleTrigger asChild>
                  <Button variant="ghost" size="sm">
                    高级编辑未转义 GeoJSON
                  </Button>
                </CollapsibleTrigger>
                <CollapsibleContent>
                  <Textarea
                    key={currentKey}
                    aria-label="高级 GeoJSON 编辑"
                    className={styles.editor}
                    value={editor}
                    disabled={busy}
                    onChange={(event) => setCustomText(event.target.value)}
                  />
                  <Button
                    type="button"
                    size="sm"
                    disabled={busy || !customValid}
                    onClick={applyCustomText}
                  >
                    采用高级结果
                  </Button>
                </CollapsibleContent>
              </Collapsible>
            )}
          </section>
        </div>
      )}
      <div className={styles.actions}>
        <span>剩余冲突 {data?.total ?? "…"}</span>
        <Button variant="outline" disabled={busy} onClick={close}>
          关闭
        </Button>
        {!readOnly && (
          <Button
            disabled={
              busy ||
              !customValid ||
              customDirty ||
              (mode === "resolve"
                ? edits.length === 0
                : !canRebase && edits.length === 0)
            }
            onClick={() => void submit()}
          >
            {mode === "rebase" && canRebase ? "更新基准" : "保存解决"}
          </Button>
        )}
      </div>
    </Modal>
  );
}

function FeatureColumn({
  title,
  feature,
}: {
  title: string;
  feature: Conflict[ConflictSide];
}) {
  return (
    <article className={styles.column}>
      <h3>{title}</h3>
      <pre className={styles.code}>{sideText(feature)}</pre>
    </article>
  );
}
function fieldValue(feature: Conflict[ConflictSide], field: string): string {
  if (!feature) return "删除要素";
  if (field === "/geometry")
    return stringify(feature.geometry, undefined, 2) ?? "缺失";
  const key = propertyForPointer(field);
  if (!key || !Object.hasOwn(feature.properties ?? {}, key)) return "缺失";
  return stringify(feature.properties![key], undefined, 2) ?? "null";
}

function ComparisonTable({
  conflict,
  publishedRevision,
}: {
  conflict: LoadedConflict;
  publishedRevision: string;
}) {
  if (isWholeFeatureConflict(conflict)) {
    return (
      <div className={styles.comparison}>
        <div className={styles.comparisonHeader}>
          <strong>状态</strong>
          <strong>已发布 r{publishedRevision}</strong>
          <strong>我的草稿</strong>
        </div>
        <div className={styles.comparisonRow}>
          <strong>
            {conflict.reason === "stale_resolution"
              ? "版本已变化，请重新确认"
              : "整个要素"}
          </strong>
          <pre data-side={`已发布 r${publishedRevision}`}>
            {sideText(conflict.current)}
          </pre>
          <pre data-side="我的草稿">{sideText(conflict.draft)}</pre>
        </div>
      </div>
    );
  }
  return (
    <div className={styles.comparison}>
      <div className={styles.comparisonHeader}>
        <strong>冲突字段</strong>
        <strong>已发布 r{publishedRevision}</strong>
        <strong>我的草稿</strong>
      </div>
      {conflict.fields.map((field) => (
        <div className={styles.comparisonRow} key={field}>
          <strong>{fieldLabel(field)}</strong>
          <pre data-side={`已发布 r${publishedRevision}`}>
            {fieldValue(conflict.current, field)}
          </pre>
          <pre data-side="我的草稿">{fieldValue(conflict.draft, field)}</pre>
        </div>
      ))}
      <p>未列出的字段已按三方规则自动合并，并保留在结果预览中。</p>
    </div>
  );
}

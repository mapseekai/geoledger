# 使用指南

[项目概览](../README.md) · [API 与 SDK](api.md)

## 数据与工作区

一个项目包含数据集、成员、版本序列和工作区。正式要素由项目的 HEAD 修订定义，工作区读取固定基础修订与自己的增量。每个用户在自己的工作区中独立编辑；项目正式数据按成员权限共享。

1. `CreateProject`、`CreateDataset` 创建资源。
2. `CreateWorkspace` 返回工作区 ID、基础修订和版本 0。
3. `Save` 提交完整 GeoJSON Feature 或删除标记，携带 `expected_workspace_version`。
4. `Diff` 查看草稿差异；`Publish` 发布并创建修订。
5. `Features` 携带 `revision` 查询历史快照，`History`、`Commit` 查询发布记录。

SDK 的工作区对象自动携带草稿版本，保存时直接传 GeoJSON；SDK/CLI/HTTP 通过显式删除操作或 null 表达删除意图，保存操作提供完整要素字段。工作区单次保存 1–100 个修改，最多累计 1000 个不同要素。批量导入应分多个工作区、多个版本发布。

## 合并与冲突

同一个属性被双方修改为不同值时产生冲突；修改不同属性可以自动合并。几何作为整体比较，保留坐标顺序和 Z 值。用户通过完整最终几何表达编辑意图。

`Conflicts` 分页返回 base/current/draft 与冲突字段。`Resolve` 提交完整最终 Feature，每批最多 100 条，携带当前 HEAD 和工作区版本。`Rebase` 解决剩余冲突并更新基础修订；`Publish` 再次检查并原子发布。

发布或草稿编辑改变了已确认的上下文时，解决选择标记为 `stale_resolution`，需要再次确认，让解决结果对应最新的编辑上下文。

## 重试与分页

SDK 工作区对象会在首次发布前生成 UUID `request_id` 并保留完整请求；CLI 或直接 HTTP 调用需自行保留。发生超时、连接中断或结果未知时，使用相同 ID 和请求体重试；已提交的请求返回保存的原结果。相同 ID 配不同内容会返回冲突。SDK 由调用方明确发起写入重试，保留原请求的幂等语义。

正式要素分页固定首屏返回的 `revision`，后续页继续传该修订。草稿分页核对 `workspace_version`；工作区变化时重新读取。分页使用 `after`，每页默认 100、最大 1000；数据量大时按响应大小降低页大小。

## 撤销

`Restore(project, revision)` 创建包含指定提交反向修改的工作区。检查差异并发布后形成新修订，原提交和审计保留。

## CLI 与管理界面

`gl info`、`gl projects`、`gl history PROJECT` 提供常用查询。`gl call OPERATION --file request.json` 覆盖所有版本操作，`--file -` 从标准输入读取。请求使用业务 JSON 的 snake_case 字段名，要素直接使用 GeoJSON 对象，见 [SDK 示例](api.md#python)。

管理界面提供项目与数据集管理、工作区创建、GeoJSON 要素编辑、差异查看和版本发布。工作区页面可分页查看并解决冲突、更新基准和丢弃草稿；版本历史可撤销指定提交，成员与审计页面帮助管理权限和追踪操作。要素读取、展示、编辑和保存保持原始数字精度，包括 64 位整数。完整流程见 [控制台说明](console.md)。

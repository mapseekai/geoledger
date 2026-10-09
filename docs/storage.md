# 存储扩展接口

[项目概览](../README.md) · [开发验证](development.md)

本指南面向存储适配器开发者。现有 SQLite 与 PostGIS 后端通过相同接口提供事务、版本历史、发布收据和审计，扩展后端时沿用客户端协议和应用层合并规则。

## 分层与接入

`geoledger-server` 负责传输、认证和资源上限；`geoledger-engine::Application` 负责授权、校验、三方合并与操作编排；[repository.rs](../crates/engine/src/repository.rs) 定义存储语义。SQL、PostgreSQL 客户端和 SQLite 连接都封装在 [session 适配器](../crates/engine/src/session.rs) 中。

新增后端实现两个 trait：

- `StorageBackend: Send + Sync`：名称、初始化、健康检查，以及按期限创建读/写事务。
- `RepositoryTransaction: Send`：项目、成员、工作区、要素、历史、发布暂存区、幂等收据、审计和原子提交。

通过 `Application::with_backend(Arc<dyn StorageBackend>)` 注入。接口接收项目、数据集、修订、游标等业务参数，由适配器封装 SQL、数据库连接和传输细节。可接入 SQL 或其他具备必要事务能力的存储；服务端配置工厂增加后端选择即可，SDK 与业务规则保持一致。

## 必须实现的保证

1. 一次 Application 操作对应一个事务。读操作获得一致快照；事务对象未 commit 就被丢弃时回滚全部写入。
2. `project_head(lock=true)` 将同项目发布串行化，并在获得锁后读取最新 HEAD；`workspace_state(write=true)` 串行化同工作区写入。成员授权在事务期间稳定。
3. 正式要素、历史有效期、commit/change、HEAD、工作区状态、幂等收据、审计同事务提交。全部写入在事务内原子完成。
4. `publication_receipt` 的键是 `(project, subject, request_id)`。已提交收据不可变；成功收据随事务提交，失败事务整体回滚。提交后响应丢失可用原请求恢复结果。
5. 修订可见性为 `valid_from <= revision` 且 `valid_to` 为空或大于 revision；删除作为历史墓碑保留。
6. 要素分页先剔除被草稿覆盖的基础要素，再做 bbox 过滤；移出范围和删除的草稿也会遮蔽原要素。游标按字节顺序稳定排序，严格大于 after。
7. `begin_merge` 清空事务私有暂存区；stage 仅存放已验证的候选。append 方法原子消费同一份暂存区。候选在发布提交后形成公开历史。
8. 所有等待、连接、查询和提交服从传入期限。结果未知时返回错误，保留客户端确认路径；后端准确报告结果确定性，并保持指定存储配置。
9. 同项目 `append_audit` 必须按提交顺序分配 ID，保证 audit 游标按已提交事件的顺序前进；PostGIS 在分配 ID 前获取独立的项目级事务 advisory lock，以独立审计锁协调顺序，其他项目仍可独立提交。
10. 返回页受条数与内存限制约束；新后端必须提供能定位游标的索引，通过索引按页读取数据。

## 后端无关行类型

接口以 `Row::new(Vec<Cell>)` 返回固定投影，Cell 为 Text、Integer(i64)、Real、Bool、Null。`Option<Row>` 的 None 表示记录不存在；单元格 Null 表示该字段为空。数据库类型由适配器转换为统一 Cell。下面是实现必须遵守的投影顺序（`?` 表示可空）：

| 方法 | 返回字段顺序 |
|---|---|
| member_role | role:text, state:text（排除已移除成员与已删除项目） |
| project_head | head:i64 |
| workspace_state | base_revision:i64, version:i64, status:text |
| feature_page | feature_id:text, properties:text, geometry:text? |
| feature_at | properties:text?, geometry:text? |
| dataset_exists / commit_exists | 存在时返回单行标记 |
| advance_workspace | version:i64 |
| list_projects | project:text, name:text, head:i64, state:text, role:text |
| project_info / live_project | name:text, head:i64, state:text |
| list_members | subject:text, role:text（仅有效成员，按 subject 排序） |
| owned_projects | count:i64（subject 为有效 owner 且未删除的项目数） |
| owner_summary | owner_count:i64, target_is_owner:i64（0/1） |
| list_datasets | dataset:text, name:text |
| list_workspaces | workspace:text, base_revision:i64, version:i64, status:text |
| has_resolution | bool |
| count_deltas / count_commit_changes | count:i64 |
| diff_page | dataset:text, key:text, draft_properties:text?, draft_geometry:text?, base_properties:text?, base_geometry:text? |
| history_page | revision:i64, subject:text, message:text, created_at:text |
| commit_page | dataset:text, key:text, before_properties:text?, before_geometry:text?, after_properties:text?, after_geometry:text? |
| audit_page | id:i64, subject:text, action:text, detail:text, created_at:text |
| merge_page | dataset:text, key:text, draft_properties:text?, draft_geometry:text?, resolved_head:i64?, resolution_stale:bool, base_properties:text?, base_geometry:text?, current_properties:text?, current_geometry:text? |
| publication_receipt | payload:text, result:text |

properties/detail/payload/result 为精确 JSON 文本，geometry 为规范化后的 GeoJSON geometry 文本，空几何使用 Null。stage 的 before/after 是序列化的 `{properties: object, geometry: string|null}`，整体 Null 表示删除。merge_page 每批最多 32 行。所有其他列表遵守传入 limit。

要素属性的字符串值与对象键使用 U+0000 以外的 JSON 文本，包括嵌套对象和数组；共同应用层统一校验输入，含实际 U+0000 时返回 `invalid_argument`，保持 SQLite 与 PostGIS 的业务语义一致。普通文本中的字面反斜杠序列 `\u0000` 可以保留，它与 JSON 解码后得到的 NUL 字符不同。

## SQLite 与 PostGIS

SQLite 使用 WAL、FULL synchronous、外键、写事务 BEGIN IMMEDIATE，读事务使用 query_only 和一致快照。RTree 先筛选候选，再以共同几何实现做精确相交判断。写入排队有总期限，超过期限返回 busy/deadline。

PostGIS 使用连接池、读事务 REPEATABLE READ、发布项目行锁、工作区行锁与 deadline-aware 数据库驱动。几何的规范 GeoJSON 与属性 JSON 文本保留跨后端一致性，PostGIS 在 `ST_GeomFromGeoJSON(geom)` 上创建 GiST 表达式索引；bbox 使用 PostGIS 空间查询。几何合法性在共同应用层校验。

当前格式为 6，正式要素与历史快照统一读取 `gl_history`。新库直接执行当前结构；已有库启动时校验当前格式。新后端通过 `StorageBackend::initialize` 创建当前结构。逻辑导出与导入由 `StorageBackend::export`/`import` 提供（SQL 后端共用 [portable](../crates/engine/src/session/portable.rs) 实现，导出格式与后端无关），新后端可实现这两个方法以支持跨后端迁移。成员移除为软删除（`removed` 标记），历史、审计与收据仍引用原成员行；`ensure_identity` 为管理员操作创建不授予权限的成员行以满足审计外键。两个后端均有不可变 commit/change/audit/receipt 约束，并保护历史有效期的关闭规则。各后端使用独立文件或数据库，跨后端迁移使用 `geoledger-server export`、`import` 与 `verify`，见 [备份与恢复](production.md#备份与恢复)。

## 新后端验收

运行 [conformance](../crates/engine/tests/conformance.rs) 同一组业务场景，并增加该后端的事务失败、锁等待期限、重启和崩溃恢复测试。必须覆盖并发发布、草稿竞态、删除遮蔽、精确数字、XYZ 几何、旧解决选择失效、权限隔离、失败批次回滚、原请求重试、历史分页与不可变性。单独运行容量用例验证索引与真实数据规模。

接入步骤为实现 trait、配置服务端后端选择、运行共用 conformance 套件，再完成故障恢复和专项容量验收。构建与测试命令见 [开发指南](development.md#回归测试入口)，上线配置和备份要求见 [生产运行](production.md)。

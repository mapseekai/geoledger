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
| dataset_exists | geometry_type:text, name:text, coordinate_dimension:i32 |
| commit_exists | 存在时返回单行标记 |
| advance_workspace | version:i64 |
| list_projects | project:text, name:text, head:i64, state:text, role:text |
| project_info / live_project | name:text, head:i64, state:text |
| list_members | subject:text, role:text（仅有效成员，按 subject 排序） |
| owned_projects | count:i64（subject 为有效 owner 且未删除的项目数） |
| owner_summary | owner_count:i64, target_is_owner:i64（0/1） |
| list_datasets | dataset:text, name:text, geometry_type:text, coordinate_dimension:i32 |
| list_workspaces | workspace:text, base_revision:i64, version:i64, status:text |
| has_resolution | bool |
| count_deltas / count_commit_changes | count:i64 |
| diff_page | dataset:text, key:text, draft_properties:text?, draft_geometry:text?, base_properties:text?, base_geometry:text? |
| history_page | revision:i64, subject:text, message:text, created_at:text, source_workspace:text |
| commit_page | dataset:text, key:text, before_properties:text?, before_geometry:text?, after_properties:text?, after_geometry:text? |
| audit_page | id:i64, subject:text, action:text, detail:text, created_at:text |
| merge_page | dataset:text, key:text, draft_properties:text?, draft_geometry:text?, resolved_head:i64?, resolution_stale:bool, base_properties:text?, base_geometry:text?, current_properties:text?, current_geometry:text? |
| publication_receipt | payload:text, result:text |

properties/detail/payload/result 为精确 JSON 文本，geometry 为规范化后的 GeoJSON geometry 文本，空几何使用 Null。stage 的 before/after 是序列化的 `{properties: object, geometry: string|null}`，整体 Null 表示删除。merge_page 每批最多 32 行。所有其他列表遵守传入 limit。

要素属性的字符串值与对象键使用 U+0000 以外的 JSON 文本，包括嵌套对象和数组；共同应用层统一校验输入，含实际 U+0000 时返回 `invalid_argument`，保持 SQLite 与 PostGIS 的业务语义一致。普通文本中的字面反斜杠序列 `\u0000` 可以保留，它与 JSON 解码后得到的 NUL 字符不同。

## SQLite 与 PostGIS

SQLite 使用 WAL、FULL synchronous、外键、写事务 BEGIN IMMEDIATE，读事务使用 query_only 和一致快照。RTree 先筛选候选，再以 SpatiaLite 的 ST_Intersects 做精确相交判断。写入排队有总期限，超过期限返回 busy/deadline。

PostGIS 使用连接池、读事务 REPEATABLE READ、发布项目行锁、工作区行锁与 deadline-aware 数据库驱动。`geom` 使用原生 `geometry` 类型（SRID 4326，兼容 XY/XYZ），直接创建 GiST 索引；bbox 直接查询几何列，不在查询时解析 GeoJSON。几何结构在共同应用层校验，自相交等拓扑问题仍作为警告，保持原始坐标。

当前格式为 11，正式要素与历史快照统一读取 `gl_history`。新库直接执行当前结构；已有库启动时校验当前格式。新后端通过 `StorageBackend::initialize` 创建当前结构。逻辑导出与导入由 `StorageBackend::export`/`import` 提供（SQL 后端共用 [portable](../crates/engine/src/session/portable.rs) 实现，导出格式与后端无关），新后端可实现这两个方法以支持跨后端迁移。成员移除为软删除（`removed` 标记），历史、审计与收据仍引用原成员行；`ensure_identity` 为管理员操作创建不授予权限的成员行以满足审计外键。两个后端均有不可变 commit/change/audit/receipt 约束，并保护历史有效期的关闭规则。各后端使用独立文件或数据库，跨后端迁移使用 `geoledger-server export`、`import` 与 `verify`，见 [备份与恢复](production.md#备份与恢复)。

SQLite 必须加载 SpatiaLite 5 扩展，使用注册的 `geom`（XY）和 `geom_z`（XYZ）几何 BLOB 列与各自的标准 RTree 索引；两个维度共用业务表，单个要素按维度写入对应列。数据集创建时声明不可变的 `coordinate_dimension`（2 或 3，默认 2），所有要素坐标必须匹配，混合维度返回明确的参数错误。嵌套集合和集合中的多部件几何仅在派生的空间表示中展平，快照保持原样。扩展加载失败时启动失败。默认加载 `mod_spatialite`，可通过 `GL_SPATIALITE_EXTENSION` 指定管理员安装的可信库路径。扩展加载后立即关闭动态加载权限。

两个后端的 `geometry_json` 保留原始几何快照，用于精确响应、版本比较和逻辑导出；空间过滤使用原生几何列。SpatiaLite 不表示空几何集合，因此这类要素仅保留快照、不进入空间索引，仍参与普通分页与计数。导入在同一事务内重建原生几何与索引；备份包含原生几何、空间元数据与索引。格式 11 直接初始化新库，部署使用匹配当前格式的数据目录。

## 新后端验收

运行 [conformance](../crates/engine/tests/conformance.rs) 同一组业务场景，并增加该后端的事务失败、锁等待期限、重启和崩溃恢复测试。必须覆盖并发发布、草稿竞态、删除遮蔽、精确数字、XYZ 几何、旧解决选择失效、权限隔离、失败批次回滚、原请求重试、历史分页与不可变性。单独运行容量用例验证索引与真实数据规模。

接入步骤为实现 trait、配置服务端后端选择、运行共用 conformance 套件，再完成故障恢复和专项容量验收。构建与测试命令见 [开发指南](development.md#回归测试入口)，上线配置和备份要求见 [生产运行](production.md)。

数据集必须声明不可变的 `geometry_type`（`point`、`line`、`polygon`）。单几何和多几何归入同一类；GeometryCollection 的所有成员必须同类，null 几何表示暂未提供位置。保存、冲突解决、重设基线和发布均执行类型校验。

删除项目清除全部数据集、工作区、版本、历史快照和发布收据，保留项目删除标记、成员引用与不可变审计记录。删除数据集仅清除对应记录：共享版本保留其他数据集的变更，共享工作区保留并递增版本、失效旧冲突解决标记；仅因此变空的版本和工作区会移除。未涉及的空工作区不受影响。项目 head 保留为已分配版本号的高水位，版本号可以有空缺，不重新编号或复用。已删除版本的发布收据会清除，幸存版本的原始请求收据保持不变。

`RepositoryTransaction::purge_data` 在项目锁保护下启用并清除事务内的 `gl_purge` 标记，允许指定项目删除不可变数据记录。其他写入仍受不可变触发器约束；审计不允许删除。此标记不导出、不持久保留，失败事务同时回滚标记与数据删除。

## 已有业务表

PostGIS 可以在已有业务数据库中初始化 GeoLedger 版本表，并将业务表绑定为数据集。指定 schema、表名、主键列和几何列，系统分页建立初始快照与首个提交。平台管理员需要具备项目成员身份，数据库服务账号需要读取、增删改及表所有权或等效的所有者角色权限，以执行锁表和触发器的创建、启用与删除；仅授予 `TRIGGER` 权限不足。

业务表使用单列主键（整数、文本或 UUID），以及声明类型的 `geometry(Point/LineString/Polygon/Multi*,4326)` 几何列，坐标为 XY 或 XYZ。其余普通可写列映射为要素属性，保留整数精度；新增与修改提交完整属性集合。表使用完整行可见性，属性列由应用写入，独立普通表的主键负责标识唯一性。每张原表绑定一个数据集。普通主键与可延迟主键均可用于增删改发布。

工作区草稿与历史快照存入 GeoLedger 版本表。发布获取业务表写锁，将属性、几何和增删改与版本记录放在同一个事务中；全部参与表完成写入后统一校验延迟约束，可在同次发布中创建相互引用的记录；原表约束失败时整体回滚。业务触发器执行后的最终表状态也会与待发布结果核对。纳管表的普通 SQL 写入通过保护触发器要求走 GeoLedger；数据库管理员管理表结构和权限，并保留该保护。发布检查表名、列结构和当前内容，防止意外替换表或越过保护后覆盖数据。

格式 11 使用事务内变更记录跟踪原表的旧、新主键，正常增量发布按主键核对涉及的行。业务触发器产生的额外修改也参与核对，延迟触发器在提交前执行并校验。表恢复、跟踪触发器状态变化或 TRUNCATE 会触发完整内容检查；检查成功后刷新校验状态。SQLite 范围分页根据有界空间候选探测选择主键分页或 RTree 候选查询，兼顾局部和全图浏览。草稿冲突选择通过部分索引完成失效标记；单条草稿操作显式使用当前主键索引，空间候选先按 rowid 取键再按主键读取，保证数据库统计信息陈旧时仍使用索引路径。复测方法见 [增量性能测试](performance.md#已有大数据集上的增量增删改)。

`postgis_source` 保存本地绑定元数据。PostgreSQL 原生全库备份包含原表、绑定和触发器，适合完整恢复。逻辑 `export` 保存可移植版本数据，`import` 将其恢复为独立的内部数据集；绑定元数据与业务表由原数据库管理，这样跨环境导入具有独立的写入目标。删除 GeoLedger 数据集或项目解除绑定，原表保留。绑定标识随触发器备份恢复；业务表增加列、重命名或删除后，仍可清理对应绑定，原位置的其他同名表保持独立。

SQLite/SpatiaLite 将要素、原生几何和历史保存在当前数据库；外部 GeoJSON 文件可以在创建数据集时导入。

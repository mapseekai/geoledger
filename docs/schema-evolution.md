# 字段结构版本管理

支持两种入口，全部经过 Application 和同一个 journal / marker 恢复协议。新导入的数据集使用 schema v2；已有 v1 数据集需要先升级。

## 工具命令

下列 `gl` 代表 `./target/release/gl --repo /path/to/repo`，连接通过 `GL_DATABASE_URL` 提供。

```bash
gl upgrade
gl add-field roads note --type text -m '新增备注'
gl rename-field roads note memo -m '备注字段改名'
gl alter-field-type roads memo --type 'varchar(200)' -m '调整字段类型'
gl drop-field roads memo --discard -m '删除备注'
gl schema roads --reference HEAD
```

每条字段命令要求整个工作副本干净并自动提交。删除必须带 `--discard`。命令中的 DDL、记录扫描、dirty 清理和数据库标记在同一个 PostgreSQL 事务内完成；类型转换失败、约束失败或不支持的定义会回滚，不发布新 HEAD。

## 外部 SQL / QGIS 编辑

```sql
ALTER TABLE public.roads ADD COLUMN note text DEFAULT '待核对';
ALTER TABLE public.roads RENAME COLUMN note TO memo;
ALTER TABLE public.roads ALTER COLUMN memo TYPE varchar(200) USING memo::varchar(200);
```

随后执行 `gl status`、`gl diff`、`gl commit -m '调整字段结构'`。也支持外部 `DROP COLUMN`。稳定字段 ID 配合 PostgreSQL 表 OID / attnum 识别改名；删除后同名新增会得到不同 ID。多个受支持的 DDL 可以合并为一次提交，并可包含同时发生的数据编辑。

`status` / 工作副本 `diff` 的 `schema_changes` 给出结构前后值。发生结构变化的数据集暂不逐行计算预览差异，而是标记 `requires_full_scan: true`；`record_counts_complete: false` 表示记录统计不完整，不能把显示的 0 当成没有数据变化。提交会完整扫描这些数据集，返回 `changed_records: null`、`rescanned_records` 及其他数据集的 `incremental_changed_records`。历史版本 diff 流式计数并返回精确记录变化总数，按 limit 和约 8 MiB 载荷预算解码预览。status/diff 的返回条数可能小于 limit；以 truncated 判断是否完整。单条大记录可超过预算。

外部 DDL 已经由外部事务提交。工具可以拒绝记录不支持的变更，但不能撤销外部事务；此时需先在数据库中修正不支持的结构。不要绕过触发器、改动 `_spatial_version` 元数据或删除重建注册表。

## 支持范围

| 操作 | 支持与限制 |
|---|---|
| 新增 | 工具添加可空字段；外部 DDL 可添加适配器支持的字段和可还原的字面量默认值。非空字段必须已有合法数据/默认值 |
| 删除 | 普通非主键字段；不隐式删除依赖约束或索引；表达式默认值等无法安全重建的定义拒绝 |
| 改名 | 保留稳定 ID；包含引号和保留字的列名正确转义；可保留已有约束/索引的列名引用 |
| 修改类型 | 适配器支持的 PostgreSQL 内建标量及 geometry 类型；由数据库执行显式 cast，失败即回滚；不接受自定义 USING SQL |

现有字段的主键、可空性、默认定义和依赖对象不是本轮通用迁移功能。带任意表达式默认值的新列、约束/索引增删及其他不支持的 DDL 会拒绝。常见字面量默认值可恢复，序列和任意函数表达式不在此范围内。修改类型可能舍入/截断，行为遵循 PostgreSQL 对指定类型的显式转换；原始值保存在旧版本中。

schema v1/v2 不保存列 collation。非默认列 collation（包括显式 `C` 和自定义 ICU collation）在导入及后续结构验证时拒绝；外部修改成非默认 collation 后，commit、restore、reset 和字段命令也会拒绝，必须先在数据库中处理该不支持的定义。本次不改变对象编码；旧历史中从未保存过的 collation 无法自动补回，已有此类数据应结合数据库备份人工核对。

## 历史与合并

`switch`、`reset --hard`、`restore --discard`、`revert` 同时处理结构与数据。跨结构版本恢复采用保留表 OID 的事务内清空、DDL 和历史行分批回填，全程保持触发器启用。它恢复历史记录，不依赖反向类型转换，因此删除列值和转换丢失的小数可以精确找回。索引和约束兼容性会校验，不能以 CASCADE 静默删除依赖。

同结构的版本应用先分批删除全部待替换旧行，再分批回填目标行，避免唯一值交换在中间状态触发 UNIQUE 冲突；两阶段处于同一事务。丢弃工作副本修改时，在 dirty key 集合上采用相同顺序。触发器始终启用，失败会回滚全部删除和插入，journal/marker 的发布顺序保持不变。

结构变更提交和恢复是整表工作，会加表锁并产生 WAL、dirty 记录及新对象；不是常数时间的元数据操作。普通数据变更仍走增量路径。

三方合并按稳定列 ID 对齐，可处理一侧改名、另一侧修改数据以及两侧独立新增字段。同一字段的不兼容定义修改、同名不同 ID 新增、删除字段与另一侧编辑该字段（含新插入行的非空值）等冲突拒绝，不静默丢弃数据。来自旧结构的新行会应用受支持的新增字段常量默认值；已存在字段的显式 NULL 不会被默认值覆盖。自动合并前会批量验证目标类型；若规范化改变候选值（包括文本截断、舍入或规范文本变化），返回 conflict 并保持 HEAD/工作副本不变，需要先在分支上显式对齐值或字段类型。结构冲突暂未提供逐字段交互解决命令，需先在分支上对齐结构；普通记录冲突继续使用 `resolve` / `merge --continue`。

## v1 → v2 迁移计划

1. 保留旧版本仓库和数据库的一致备份，在干净的 v1 工作副本运行新程序 `upgrade`。已经发生 v1 DDL 漂移时，先还原结构；v1 不具备可靠改名识别信息。
2. 升级为当前分支创建一个元数据提交：为字段分配稳定 ID，记录列位置映射和依赖索引定义；记录树直接复用，不重新导入几何。所有旧提交和 v1 schema 对象保持不变。
3. 新结构对象采用 `schema/v2`，Schema.version 为 2；RepositoryState.version 和 SQLite user_version 升至 2。`record/v1`、`tree-node/v1`、`snapshot/v1`、`commit/v1` 编码保持原样。压缩器复用、缓存和 SQL 批处理不改变对象 ID 协议。
4. 在准备可能提交数据库的恢复日志时就持久化仓库格式 2，防止旧程序介入未完成的结构操作。旧程序将拒绝该仓库。即便操作后来未完成，也不要手改版本号降级，使用新程序 `recover`。
5. 新程序可读取两代历史。切换到尚未升级的旧分支后，若要进行新结构变更，先再次 `upgrade` 该分支。已经升级且无变更时命令幂等返回。无原地降级工具；回退程序版本须恢复配套备份，不能只替换二进制。

新导入直接建立 v2 结构；已有 v1 仓库的普通数据操作仍可使用新程序，不强制先升级。HTTP `/v1/commands` 与 gRPC `Execute` 支持相同命令；这次没有新增专用 Protobuf 字段操作方法，详见 [API](api.md)。

# 字段结构版本管理

GeoLedger 支持通过工具命令及外部 SQL / QGIS 编辑字段结构，两种方式共用 Application 和 journal / marker 恢复协议。新导入的数据集直接获得稳定字段 ID 和当前格式的 schema 对象。

## 工具命令

配置实际的 `GL_DATABASE_URL`，在已初始化并导入 roads 的仓库执行：

```bash
gl --repo ./demo-repo add-field roads note --type text -m '新增备注'
gl --repo ./demo-repo rename-field roads note memo -m '备注字段改名'
gl --repo ./demo-repo alter-field-type roads memo --type 'varchar(200)' -m '调整字段类型'
gl --repo ./demo-repo drop-field roads memo --discard -m '删除备注'
gl --repo ./demo-repo schema roads --reference HEAD
```

每条字段命令要求干净工作副本，默认作者为 `mapseekai`，并自动提交。删除字段通过 `--discard` 确认。DDL、记录扫描、dirty 清理和数据库标记位于同一 PostgreSQL 事务中；类型与约束检查通过后发布 HEAD，失败时按事务回滚。

## 字段能力与准备条件

新增命令创建可空字段，外部 DDL 可添加具有可恢复字面量默认值的字段；非空字段准备有效数据或默认值。删除适用于普通非主键字段，RESTRICT 语义保护依赖关系。改名保存稳定 ID，正确转义列名并校验相关引用。修改类型使用适配器声明的 PostgreSQL 标量或 geometry 类型，由数据库执行显式 cast。

适用表采用单列主键和默认列排序规则。列名、增删和类型演进与现有主键、可空性、默认定义及依赖对象共同接受一致性校验。类型转换遵循 PostgreSQL 的显式转换规则，转换前原值保存在历史中。

## 外部编辑与统计

外部 SQL / QGIS 可执行 ADD COLUMN、DROP COLUMN、RENAME COLUMN 和 ALTER COLUMN TYPE，再通过 `gl --repo ./demo-repo status`、`diff` 和 `commit` 记录。稳定字段 ID 配合表 OID / attnum 识别改名；同名删除再新增会获得新 ID。一次提交可以包含多个字段操作及记录编辑。外部 DDL 由调用它的数据库事务提交，GeoLedger 随后读取并校验定义；操作期间保持 `_geoledger` 元数据、表绑定和跟踪触发器完整。

`schema_changes` 返回结构前后值。结构变化的数据集使用 `requires_full_scan: true` 和 `record_counts_complete: false` 标记整表扫描阶段，提交返回 `changed_records: null`、`rescanned_records` 与其他数据集的 `incremental_changed_records`。历史 diff 流式统计精确总数，并按条数和约 8 MiB 载荷预算解码预览。客户端读取 `total`、`truncated` 和 `record_counts_complete` 判断结果范围，单条大记录可超过预算。

## 历史恢复

`switch`、`reset --hard`、`restore --discard`、`revert` 同时处理结构和数据。跨结构恢复保留表 OID，在事务内清空、调整定义、分批回填历史对象。跟踪触发器持续启用，索引和约束在操作前后校验，历史值直接来自对象记录。

同结构恢复先分批删除待替换旧行，再分批回填目标行，两阶段位于同一事务，可完成唯一值交换。丢弃未提交修改时使用相同顺序。结构扫描和回填按整表处理，并占用相应锁、WAL 和对象空间；普通数据提交采用增量路径。

## 三方合并

结构按稳定 ID 对齐，支持一侧改名与另一侧数据编辑、两侧独立新增字段。新增字段投影使用适配器提供的常量默认值，原有字段的显式 NULL 按原值保留。

同字段定义冲突、同名不同 ID、删除字段与另一侧编辑该字段等情况返回冲突，HEAD 和工作副本保持在合并前状态。先在分支中对齐结构或值后可再次合并；记录冲突使用 `resolve` 和 `merge --continue` 完成。候选记录在发布前按目标类型批量校验，值表达一致后统一应用。

统一格式见 [数据格式与初始化](format.md)，HTTP / gRPC / Thrift 的调用方式见 [API](api.md)。

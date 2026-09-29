# GeoLedger 数据格式与初始化

## 统一命名

项目与 Cargo 包使用 `geoledger` / `geoledger-*`，命令行为 `gl`，默认作者为 `mapseekai`。配置变量为 `GL_DATABASE_URL`、`GL_AUTHOR`、`GL_STATEMENT_TIMEOUT_SECS`、`GL_API_TOKEN`、`GL_TEST_DATABASE_URL`。本地元数据目录为 `.geoledger`，PostgreSQL 元数据 schema 为 `_geoledger`，字段身份键为 `geoledger.column-id`。gRPC / Thrift 使用 `geoledger.v1` 命名空间和 `GeoLedger` 服务。

## 当前格式

持久化对象、字段结构、仓库状态、SQLite 和 PostGIS 跟踪统一使用 **格式 3**。核心版本定义见 [模型](../crates/core/src/model.rs)，摘要实现见 [对象接口](../crates/core/src/object.rs)。

对象域为 `geoledger\0object-v3\0`，对象类型为 `record/v3`、`schema/v3`、`tree-node/v3`、`snapshot/v3` 和 `commit/v3`。摘要由对象域、类型长度、类型和编码字节共同计算。字段结构保存稳定身份、类型、默认定义及适配器元数据。

SQLite 的 `application_id` 为 `0x474c4433`（GLD3），`user_version` 为 3。新仓库从初始化时写入当前格式；读取状态及 pending journal 时校验格式和字段身份。PostGIS 的 `_geoledger.format` 记录版本 3，跟踪索引为 `dirty_pk_c_v3`，触发器为 `gl_track_row_v3` 和 `gl_reject_truncate_v3`，定义见 [bootstrap.sql](../crates/postgis/src/bootstrap.sql)。

HTTP / gRPC / Thrift 的 API v1 独立于数据格式版本；属性和几何 codec 使用各自的编码标识。

## 开发数据初始化

本次格式切换采用重新初始化开发仓库、从源数据建立快照的方式。选择新的仓库目录，在专用开发数据库中准备新建或重新导入的表，并将实际连接串设置到 `GL_DATABASE_URL`。

```bash
gl --repo ./demo-repo init
gl --repo ./demo-repo import roads --schema public --table roads
gl --repo ./demo-repo status
gl --repo ./demo-repo fsck
```

`init` 创建当前格式仓库，`import` 建立稳定字段 ID、记录快照和数据库跟踪。初始化使用新目录可保留开发参考数据；数据清理按确认的保留范围单独进行。测试使用 `GL_TEST_DATABASE_URL` 和专用 `geoledger_test` 数据库。

回归覆盖对象域、schema 身份、SQLite 标识、状态和恢复日志、字段历史、所有入口的默认作者及显式覆盖，结果见 [验证记录](verification.md)。

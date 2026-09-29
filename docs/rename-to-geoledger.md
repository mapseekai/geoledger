# GeoLedger 重命名说明：2026-09-29

## 对外名称

项目与源码目录由 `spatial-version` 改为 `geoledger`。七个工作区包统一使用 `geoledger` / `geoledger-*`，Rust 导入使用 `geoledger` / `geoledger_*`。CLI 包为 `geoledger-cli`，可执行文件和操作前缀为 `gl`，例如 `gl init`、`gl status`、`gl commit`。子命令业务语义及 HTTP JSON 的 `op` 值不变。

配置前缀由 `SV_` 改为 `GL_`，包括 `GL_DATABASE_URL`、`GL_AUTHOR`、`GL_STATEMENT_TIMEOUT_SECS`、`GL_API_TOKEN`、`GL_TEST_DATABASE_URL`。旧环境变量不再自动读取；自定义数据库变量仍可通过 `--database-env NAME` 指定。

**重新启动服务前，必须将鉴权配置迁移为 `GL_API_TOKEN`。旧 `SV_API_TOKEN` 的存在不再代表启用了鉴权。** 非 loopback 监听仍要求至少 24 字节的令牌。PostGIS 连接的 `application_name` 改为 `geoledger`。显式数据库测试仅接受名为 `geoledger_test` 的隔离数据库，使用 `GL_TEST_DATABASE_URL`，不放宽业务数据库保护。

## gRPC 与 Thrift

gRPC IDL 改为 `crates/server/proto/geoledger.proto`，包为 `geoledger.v1`，服务为 `GeoLedger`，例如 `geoledger.v1.GeoLedger/Status`。Rust 生成模块为 `geo_ledger_client` / `geo_ledger_server`。旧 gRPC 服务路径不再注册，客户端需要重新生成或更新绑定。

Thrift IDL 改为 `crates/thrift-gen/idl/geoledger.thrift`，命名空间为 `geoledger.v1`，服务为 `GeoLedger`。请求字段、方法名和 Framed Binary 传输不变；Rust 客户端更新为 `GeoLedgerClientBuilder`。生成代码由构建脚本再生成，不手工修改。HTTP 路由、JSON 命令和存储格式版本不变。

## 既有数据兼容

新建数据仓库使用 `.geoledger/repository.sqlite`。只存在旧 `.spatial-version` 时继续原位使用，不自动迁移或复制。已有任一元数据目录时，`init` 均拒绝重新初始化；两种目录同时存在时明确报冲突，避免读错仓库或覆盖历史。

以下名称属于已落盘的格式协议，不作为品牌字符串替换：对象哈希的 v1 命名域 `spatial-version\0object-v1\0`、字段身份键 `spatial-version.column-id`、SQLite application ID `0x53565031`、PostGIS schema `_spatial_version`、触发器 `sv_track_row_v1` / `sv_reject_truncate_v1`。

保留这些名称可维持对象 ID、列身份、提交历史、跟踪与恢复标记的一致性。本次不重写历史、不关闭触发器、不执行数据库迁移。历史验证文档保留当时的命令、路径与测量结果，并标注为历史记录。

## 影响分析与回归

GitNexus 对旧工作区建立索引并分析对象摘要、存储初始化及应用层。对象摘要变更被标为 CRITICAL，因此保留已有命名域。部分 Rust 调用未解析，图谱不能替代全库文本检查、编译和测试。

新增 9 项回归测试，覆盖命令名与帮助、作者配置的离线工作流、超时与鉴权环境变量、新旧仓库目录、双目录拒绝、锁和重复初始化保护、缺失仓库不产生副作用，以及从改名前实现捕获的固定对象摘要。

## 本次实测结果

在新目录下，格式检查、Clippy（所有目标，警告视为错误）、工作区测试及所有二进制和示例构建均通过。测试结果为 **40 项通过、0 失败、28 项数据库测试 ignored**，包含新增的 9 项回归测试。第三方依赖版本与校验值未改变。

改名前真实 CLI 创建的旧格式仓库，由新 `gl` 读取后提交历史完全一致，`fsck` 通过；新 CLI 创建分支后，旧 CLI 仍可读取并通过校验。重复初始化被拒绝，原仓库未被覆盖，也未产生第二套元数据目录。`gl --version` 返回 `gl 0.1.0`，Cargo 确认新目录下的七个包和唯一 CLI `gl`。

本次未配置 `GL_TEST_DATABASE_URL`，未运行真实 PostGIS 集成测试，不将 ignored 计为通过；没有连接或修改业务数据库。验证日志与旧格式样本保存在被 Git 忽略的 `.reference/geoledger-rename-20260929/`。

# GeoLedger

GeoLedger 是 PostGIS-first 的 Rust 空间数据版本控制引擎，当前为 **0.1.0 开发版**。它结合可编辑的 PostGIS 工作副本和本地版本历史，提供记录变更、字段演进、分支合并及历史恢复能力。

命令行入口为 **`gl`**，Cargo 包使用 `geoledger` / `geoledger-*`，配置使用 `GL_*`，默认作者为 **`mapseekai`**。CLI、Rust 库、HTTP、gRPC 和 Volo Thrift 共用 Application 应用层。

## 已实现功能

| 类别 | 功能 |
|---|---|
| 数据接入 | 注册具有单列主键的 PostGIS 普通表，管理普通属性和多个 geometry 列，跟踪新增、修改和删除 |
| 字段管理 | 新增、删除、改名、修改类型；通过工具命令或外部 SQL / QGIS 编辑；结构与记录共同保存历史 |
| 历史查询 | init、import、status、diff、commit、log、show、reflog、fsck |
| 分支与合并 | 创建和切换分支、checkout 别名、快进、三方字段级合并、冲突查看与解决、继续和取消 |
| 历史恢复 | restore 恢复工作副本，reset 移动分支并恢复数据，revert 以新提交撤销单父提交 |
| 一致性 | 文件锁、数据库事务、表锁、持续启用的跟踪触发器、pending journal 和操作标记恢复 |
| 服务入口 | 同步 Rust API、HTTP JSON API、HTTP/2 + Protobuf gRPC、Volo Thrift Framed Binary |

适用表采用单列主键、默认列排序规则和适配器声明的字段类型。主键规范文本最多为 8192 UTF-8 字节。几何以 XDR EWKB 保存 SRID、Z/M 和坐标表达；属性使用数据库规范文本保存精确数值。

## 构建与安装

所有命令从包含 [Cargo.toml](Cargo.toml) 的项目根目录执行。构建环境使用 Rust、C 编译器和 protoc；Linux 的 native-tls 构建还使用 OpenSSL 开发包与 pkg-config。Cargo 声明 Rust 1.88 下限，本轮验证使用 Rust 1.92.0；依赖由 [Cargo.lock](Cargo.lock) 固定。macOS 可通过 `brew install protobuf` 配置 protoc。

```bash
cargo build --workspace --bins --examples --locked
./target/debug/gl --version
cargo install --path crates/cli --locked
```

## 初始化与版本操作

先准备专用开发数据库、PostGIS 扩展和 `public.roads` 开发表。数据库角色需要创建 `_geoledger` schema、安装触发器以及读写表和元数据的权限。将实际连接串设置到 `GL_DATABASE_URL`，配置模板见 [.env.example](.env.example)。后续示例使用已安装的 `gl`。

```bash
export GL_AUTHOR=mapseekai
gl --repo ./demo-repo init
gl --repo ./demo-repo import roads --schema public --table roads
gl --repo ./demo-repo status
# 通过 SQL / QGIS 编辑注册表后执行：
gl --repo ./demo-repo diff
gl --repo ./demo-repo commit -m '更新道路属性'
gl --repo ./demo-repo log
gl --repo ./demo-repo branch draft
gl --repo ./demo-repo switch draft
```

`demo-repo` 由 `init` 创建，版本对象保存在 `.geoledger/repository.sqlite`。当前对象、schema、仓库和数据库跟踪统一使用格式 3，开发数据准备步骤见 [数据格式与初始化](docs/format.md)。连接串通过环境变量或 Rust provider 参数传入，`--database-env NAME` 可指定变量名称。CLI 作者优先级为 `--author`、`GL_AUTHOR`、`mapseekai`。

分支切换和合并以干净工作副本为前提，并在事务中更新注册表。同一仓库共用一个 PostGIS 工作副本。引用可以使用 `HEAD`、分支名或完整 64 位十六进制提交 ID。

## 字段结构与历史恢复

`add-field`、`rename-field`、`alter-field-type`、`drop-field --discard` 支持普通字段演进，每条命令自动提交。外部 SQL / QGIS 的字段编辑可通过 `status`、`diff` 和 `commit` 纳入历史。稳定字段 ID 用于识别改名和三方结构对齐。示例与准备条件见 [字段结构版本管理](docs/schema-evolution.md)。

`restore --discard` 确认丢弃未提交修改并恢复为 HEAD；`reset COMMIT_ID --hard` 确认移动当前分支并覆盖工作副本；`revert COMMIT_ID` 以新提交撤销单父提交。COMMIT_ID 使用 `gl log` 返回的完整 ID。跨结构恢复保留表 OID 和跟踪触发器，在事务中调整定义并回填历史记录。

独立字段编辑和相同结果的并发编辑可自动合并。几何按完整字段值参与合并。冲突通过 `conflicts` 查看，使用 `resolve DATASET KEY --take ours|theirs|base|delete|custom` 解决，然后执行 `merge --continue`；取消使用 `merge --abort`。自定义记录由用户准备 JSON 文件并通过 `--record` 传入，随后进行数据库类型规范化。

## HTTP、gRPC 与 Thrift

```bash
export GL_API_TOKEN="$(openssl rand -hex 24)"
gl --repo ./demo-repo serve --http 127.0.0.1:7878 --grpc 127.0.0.1:7879
```

以下地址由上一条命令创建，在服务运行时从另一个终端使用同一令牌调用：

```bash
curl -H "Authorization: Bearer $GL_API_TOKEN" http://127.0.0.1:7878/v1/status
grpcurl -plaintext -H "Authorization: Bearer $GL_API_TOKEN" \
  -import-path crates/server/proto -proto geoledger.proto \
  -d '{"limit":20}' 127.0.0.1:7879 geoledger.v1.GeoLedger/Status
```

Thrift 通过 `gl --repo ./demo-repo serve-thrift --thrift 127.0.0.1:7880` 单独启动。该服务运行时，可用 `cargo run -p geoledger-server --example thrift_client -- 127.0.0.1:7880` 调用。服务默认绑定 loopback，跨机器访问配置至少 24 字节的 Bearer token，并通过 TLS 反向代理保护传输。令牌授权范围为绑定仓库的读写及恢复操作。

gRPC 和 Thrift 均提供 15 个方法，`Execute` / `execute` 接受统一 JSON 命令，响应使用 `JsonReply.json`。详细路由、IDL、作者规则和调用方式见 [API 文档](docs/api.md)。Rust 嵌入方式见 [库示例](crates/app/examples/library.rs)，异步宿主通过 `spawn_blocking` 调用同步 Application。

## 批处理与验证

导入按主键排序流式读取；dirty 主键按游标分页；记录以约 8 MiB 分批处理；对象树差异跳过共享子树。列表最多返回 1000 项，预览同时采用约 8 MiB 载荷预算，通过 `total` / `truncated` 表达统计与输出范围。`GL_STATEMENT_TIMEOUT_SECS` 或 `--statement-timeout-secs` 配置每条事务内 SQL 的超时，默认 120 秒，范围 1–2147483 秒；锁等待为 5 秒。

检查脚本使用 Python 3。运行 `./scripts/check.sh` 执行文档、格式、Clippy 和工作区检查。设置指向专用 `geoledger_test` 数据库的 `GL_TEST_DATABASE_URL` 后，脚本显式运行 PostGIS 测试。当前结果见 [验证记录](docs/verification.md)，历史规模测量见 [大数据验证](docs/big-data-verification.md)。

## 代码与文档

[core](crates/core) 提供版本模型和树算法，[storage](crates/storage) 提供 SQLite 对象库，[postgis](crates/postgis) 提供数据库适配，[app](crates/app) 编排版本操作，[server](crates/server) 提供网络入口，[thrift-gen](crates/thrift-gen) 生成协议绑定，[cli](crates/cli) 提供 gl 命令。

参见 [架构与一致性](docs/architecture.md)、[正确性与性能改进](docs/rust-review-fixes.md)、[源码参考](docs/kart-reference.md)及[开发计划](docs/roadmap.md)。项目源码使用 [MIT 许可证](LICENSE)。

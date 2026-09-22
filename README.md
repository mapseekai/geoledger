# spatial-version

PostGIS-first 的 Rust 空间数据版本控制引擎，实验性版本 **0.1.0**。

本工作区提供 Rust 库 API、CLI、HTTP JSON API、gRPC 四种入口；它们共用同一个应用层，而不是相互调用命令行。当前版本是可运行、可测试的基础实现，不是生产级 Kart 替代品，也不兼容 Git/Kart 仓库格式。

> 项目根目录为 `/Users/zhang/code/spatial-version`，也是唯一的 Cargo workspace 根目录。下面所有命令均在项目根目录执行。

## 能力

| 类别 | 已实现 |
|---|---|
| 数据 | 注册现有 PostGIS 普通表；单列主键；普通属性、多个 geometry 列；增删改跟踪 |
| 字段 | 新增、删除、改名、修改类型；工具命令或外部 DDL；结构与数据一起提交和回退 |
| 历史 | init、import、status、diff、commit、log、show、reflog、fsck |
| 分支 | branch、switch / checkout；单仓库共享一个数据库工作副本 |
| 回退 | restore 丢弃未提交修改；reset --hard 移动分支并还原；revert 生成反向提交 |
| 合并 | 快进、三方字段级合并、冲突列表、选择/自定义解决、继续、取消 |
| 恢复 | 数据库操作标记与本地 pending journal；recover 检查并协调未完成操作 |
| 服务 | CLI、同步 Rust API、HTTP API、真正基于 HTTP/2 + Protobuf 的 gRPC |

字段变更的支持范围和旧仓库升级步骤见 [字段结构版本管理](docs/schema-evolution.md)。本轮正确性与内存/算法修复见 [Rust 审查修复记录](docs/rust-review-fixes.md)。不支持：主键结构修改、任意约束/默认表达式迁移、复合主键、identity / generated 列、外键、RLS、分区表、用户自定义触发器、远程 push/pull、Git 兼容、栅格/点云读写、几何顶点或拓扑自动合并、暂存区、对象垃圾回收。检测到不支持的表结构会拒绝。

## 目录与依赖方向

```text
crates/core       数据模型、对象协议、持久化树、DAG、diff / merge、适配器接口
crates/storage    SQLite WAL 对象库、Zstd、文件锁、仓库状态、恢复日志
crates/postgis    表结构识别、EWKB 编码、脏主键触发器、数据库事务
crates/app        统一 Application / Command；提交、分支、回退、合并编排
crates/server     Axum HTTP API、Tonic gRPC、鉴权和阻塞任务隔离
crates/cli        Clap 命令行与服务启动
```

core 不依赖 PostGIS、SQL、HTTP 或 gRPC。未来的数据格式通过 `WorkingCopyProvider` / `WorkingCopyTransaction` 扩展；存储通过 `ObjectStore` 扩展。`DatasetKind::Raster/PointCloud` 和 `Cell::Blob` 只是预留模型，尚无相应适配器。

## 构建

需要 Rust、C 编译环境与 `protoc`；Linux 使用 native-tls 时还需要 OpenSSL 开发包和 pkg-config。Cargo 声明 Rust 1.88 下限，本次实际验证使用 Rust 1.92.0，尚未验证 1.88。

```bash
cd /Users/zhang/code/spatial-version
cargo build --workspace
./target/debug/spatial-version --help
```

源码自带 Cargo.lock，应保留用于重现依赖。macOS 缺少 protoc 时可使用 `brew install protobuf`；本次开发机器已安装。

## 首次使用：先使用测试数据库

工具会为注册的表安装触发器，并在数据库中建立 `_spatial_version` 元数据 schema。数据库角色必须能够创建该 schema、安装触发器，以及读写相应表和元数据。**不要先接生产库。** 初版不是透明地支持所有已有 PostGIS 表。

```bash
export SV_DATABASE_URL='postgresql://USER:PASSWORD@HOST:5432/DATABASE'
export SV_AUTHOR='Alan'

./target/debug/spatial-version --repo ./demo-repo init
./target/debug/spatial-version --repo ./demo-repo import roads --schema public --table roads
./target/debug/spatial-version --repo ./demo-repo status
```

连接串只从环境变量或 Rust provider 参数获取，不持久化进仓库。`--database-env NAME` 可改环境变量名称。数据仓库存储在指定目录的 `.spatial-version/repository.sqlite`，不是源代码 Git 仓库。

大数据导入可显式设置 `SV_STATEMENT_TIMEOUT_SECS=900` 或全局参数 `--statement-timeout-secs 900`（秒）。默认仍为 120 秒，允许 1–2147483 秒，不能用 0 禁用。设置对当前 CLI 操作或 `serve` 创建的 provider 生效，仅作用于每个数据库事务中的单条 SQL；包含流式读取等待客户端处理的时间，不是整个命令的总超时。Rust API 使用 `PostgisProvider::with_statement_timeout(Duration)`。锁等待超时仍为 5 秒；提高语句超时可能延长注册表被锁定的时间。

随后可通过 QGIS / SQL 编辑注册表：

```bash
./target/debug/spatial-version --repo ./demo-repo diff
./target/debug/spatial-version --repo ./demo-repo commit -m '更新道路属性'
./target/debug/spatial-version --repo ./demo-repo branch draft
./target/debug/spatial-version --repo ./demo-repo switch draft
# 在数据库工作副本编辑并提交 draft 后，再切换回 main。
./target/debug/spatial-version --repo ./demo-repo switch main
./target/debug/spatial-version --repo ./demo-repo merge draft
```

存在未提交修改时，switch / merge 默认拒绝。每个仓库只有一个工作副本；switch 会实际修改注册表中的数据，不能让不同用户同时把同一组表当成不同分支。

字段命令要求干净工作副本，每次操作自动生成提交：

```bash
# 旧 v1 仓库先在干净状态升级；新导入的数据集已经使用 v2。
./target/debug/spatial-version --repo ./demo-repo upgrade
./target/debug/spatial-version --repo ./demo-repo add-field roads note --type text
./target/debug/spatial-version --repo ./demo-repo rename-field roads note memo
./target/debug/spatial-version --repo ./demo-repo alter-field-type roads memo --type 'varchar(200)'
./target/debug/spatial-version --repo ./demo-repo drop-field roads memo --discard
```

也可通过 SQL / QGIS 执行受支持的字段 DDL，再运行 `status`、`diff` 和 `commit`。`schema roads --reference HEAD` 查看某版本的结构。结构变更提交会扫描整个数据集；跨结构版本切换会重建表内数据并保留跟踪触发器，需预留执行时间和空间。

## 合并与回退

不同字段的独立修改可自动合并；相同字段变成相同结果也可合并。同字段改成不同值、删除与修改、冲突的同主键新增会保留冲突。几何作为原子字段：两侧不同的几何修改必须解决，不进行空间拓扑拼接。

```bash
./target/debug/spatial-version --repo ./demo-repo conflicts
./target/debug/spatial-version --repo ./demo-repo resolve roads 1001 --take theirs
./target/debug/spatial-version --repo ./demo-repo merge --continue
# 或取消整个未完成合并：
./target/debug/spatial-version --repo ./demo-repo merge --abort
```

有冲突时，合并候选结果和解决状态保存在本地仓库，数据库不提前写入候选结果。解决全部冲突后，continue 才统一写入并提交。`--take custom --record resolution.json` 支持完整记录自定义解决；记录值会先经过数据库类型转换校验。

```bash
# 会丢弃未提交修改，必须显式确认参数：
./target/debug/spatial-version --repo ./demo-repo restore --discard
# 保留历史，通过新提交撤销某个普通提交：
./target/debug/spatial-version --repo ./demo-repo revert COMMIT_ID
# 移动当前分支并覆盖工作副本，谨慎使用：
./target/debug/spatial-version --repo ./demo-repo reset COMMIT_ID --hard
```

引用支持 HEAD、分支名和完整 64 位十六进制提交 ID；暂不支持缩写 ID、HEAD~1、标签或 detached HEAD。revert 暂不支持根提交和多父合并提交。

## HTTP 与 gRPC

```bash
export SV_API_TOKEN='replace-with-at-least-24-random-characters'
./target/debug/spatial-version --repo ./demo-repo serve \
  --http 127.0.0.1:7878 --grpc 127.0.0.1:7879
```

默认仅监听 loopback。非 loopback 必须配置至少 24 字节的令牌。服务没有内置 HTTPS/TLS、多用户 RBAC 或租户隔离；跨机器部署必须另行配置 TLS 反向代理。令牌授权的是整个已绑定仓库，包括写入和回退，不是只读令牌。

```bash
curl -H "Authorization: Bearer $SV_API_TOKEN" http://127.0.0.1:7878/v1/status
curl -H "Authorization: Bearer $SV_API_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"op":"log","reference":"HEAD","limit":20}' \
  http://127.0.0.1:7878/v1/commands

grpcurl -plaintext -H "Authorization: Bearer $SV_API_TOKEN" \
  -import-path crates/server/proto -proto spatial_version.proto \
  -d '{"limit":20}' 127.0.0.1:7879 spatial.version.v1.SpatialVersion/Status
```

gRPC 常用操作提供类型化请求；当前返回 `JsonReply { json: string }`，尚未将动态数据和所有响应设计成完整 Protobuf 数据模型。`Execute` 可以调用全部 Command。详见 [API 文档](docs/api.md)。

Rust 嵌入示例见 `crates/app/examples/library.rs`。Application 是同步 API；异步程序必须放进阻塞线程，提供的服务和 CLI 已使用 `spawn_blocking`。

gRPC Rust 客户端示例：`cargo run -p spatial-version-server --example client -- http://127.0.0.1:7879`。

## 验证与限制

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
# 仅连接名为 spatial_version_test 的隔离数据库：
SV_TEST_DATABASE_URL='postgresql://...' \
  cargo test -p spatial-version --test postgis -- --ignored --test-threads=1
# 含事务级超时测试在内的完整检查：
SV_TEST_DATABASE_URL='postgresql://...' ./scripts/check.sh
```

PostGIS 测试默认明确标记为 ignored，不会自动连接数据库；显式运行时要求测试库名称匹配，建立随机 schema，测试数据保留在隔离库内。具体实测结果与未测事项见 [验证记录](docs/verification.md)。

初版会锁定注册表，防止提交/切换期间外部写入；这会阻塞写入，不适合直接套在高并发业务主表上。初始导入按主键排序并流式读取；dirty 主键按游标分页，记录按约 8 MiB 分批读取，普通提交/恢复逐批处理；对象树 diff 跳过相同子树并流式输出。结构合并仍可能保存较多候选键和冲突。列表最多返回 1000 项；status/diff 预览另有约 8 MiB 载荷预算，单条超大记录可超过预算，实际截断以 truncated 为准。尚无客户端完整游标分页。没有 GC，长期仓库会增长。48 万条真实数据测量见 [大数据验证记录](docs/big-data-verification.md)，尚无百万/千万要素性能保证。

`fsck` 检查仓库对象、提交图和数据树，不等于全表核对数据库。被特权用户绕过的触发器写入、数据库备份恢复、任意 DDL 和进程崩溃窗口仍需要更完整的故障注入及审计测试。现在只应作为开发验证版使用。

参见 [架构与一致性](docs/architecture.md)、[Kart 源码参考](docs/kart-reference.md)、[后续工作](docs/roadmap.md)。

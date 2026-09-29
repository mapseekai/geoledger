# API v1

## Rust 库

依赖 `crates/app` 的包 `geoledger`，使用 `Application::new(path)`、`with_provider(Arc<dyn WorkingCopyProvider>)` 和 `execute(Command)`。返回 `Result<serde_json::Value, geoledger_core::Error>`。同步数据库连接在调用线程内创建，异步宿主应通过 spawn_blocking 执行。

实际示例：`cargo run -p geoledger --example library -- ./demo-repo`。先通过 CLI 初始化并导入工作副本；示例本身只查询状态。

## 统一命令协议

HTTP `POST /v1/commands` 接受以下 JSON。相同对象可通过 gRPC 或 Thrift ExecuteRequest.command_json 传入。未知字段会拒绝。author 默认 unknown；schema 默认 public；reference/from 分支起点默认 HEAD；limit 默认 100，列表最多返回 1000 项。status/diff 预览另受约 8 MiB 载荷预算约束（单条超大记录除外），所以返回条数可以小于 limit，truncated 根据实际返回数计算；total 仍为精确记录数，结构漂移时以 record_counts_complete 为准。

```json
{"op":"init","author":"Alan"}
{"op":"import","dataset":"roads","schema":"public","table":"roads","author":"Alan","message":"初始导入"}
{"op":"upgrade"}
{"op":"schema","dataset":"roads","reference":"HEAD"}
{"op":"alter_schema","dataset":"roads","change":{"action":"add","name":"note","data_type":"text"}}
{"op":"alter_schema","dataset":"roads","change":{"action":"rename","name":"note","new_name":"memo"}}
{"op":"alter_schema","dataset":"roads","change":{"action":"alter_type","name":"memo","data_type":"varchar(200)"}}
{"op":"alter_schema","dataset":"roads","change":{"action":"drop","name":"memo","discard":true}}
{"op":"status","limit":100}
{"op":"diff","from":"main","to":"draft","limit":100}
{"op":"diff","limit":100}
{"op":"commit","message":"更新道路","author":"Alan"}
{"op":"log","reference":"HEAD","limit":100}
{"op":"show","reference":"HEAD"}
{"op":"show","reference":"HEAD","dataset":"roads","key":"1001"}
{"op":"branches"}
{"op":"branch","name":"draft","from":"HEAD"}
{"op":"switch","branch":"draft"}
{"op":"restore","discard":true}
{"op":"reset","target":"FULL_COMMIT_ID","hard":true}
{"op":"revert","target":"FULL_COMMIT_ID","author":"Alan"}
{"op":"merge","source":"draft","author":"Alan"}
{"op":"conflicts","limit":100}
{"op":"resolve","dataset":"roads","key":"1001","choice":"theirs"}
{"op":"merge_continue"}
{"op":"merge_abort"}
{"op":"recover"}
{"op":"fsck"}
{"op":"reflog","limit":100}
```

以上是独立请求示例，不是一个 JSON 数组。diff 无 to 时比较工作副本；提供 to 时比较两个历史版本。show 的 dataset 和 key 必须成对出现。成功的 merge 仍可能返回冲突状态，客户端必须检查结果，不能把 HTTP 200 等同于已生成合并提交。

自定义解决记录：

```json
{
  "op":"resolve",
  "dataset":"roads",
  "key":"1001",
  "choice":"custom",
  "record":{
    "key":"1001",
    "fields":{
      "id":{"type":"text","value":"1001"},
      "name":{"type":"text","value":"New name"},
      "geom":{"type":"null"}
    }
  }
}
```

必须提供 schema 定义的全部字段，包含主键且与 key 一致；NULL 必须符合约束。geometry 值形式为 `{"type":"geometry","value":"XDR_EWKB_LOWERCASE_HEX"}`。EWKB 不是 GeoJSON，不要传入坐标数组或 WKT；类型校验和规范化由适配器完成。Blob 是未来扩展值，PostGIS v1 不接受它。

## HTTP 路由

| 路由 | 用途 |
|---|---|
| GET /health | 服务版本与存活，配置 token 时也要求鉴权 |
| POST /v1/commands | 全部统一命令 |
| GET /v1/status?limit=100 | 工作副本状态 |
| GET /v1/log?reference=HEAD&limit=100 | 提交历史 |
| GET /v1/branches | 分支列表 |
| POST /v1/branches | {name,from} |
| GET /v1/conflicts?limit=100 | 未解决冲突 |
| POST /v1/commits | {message,author} |
| POST /v1/merges | {source,author,message} |

应用错误返回 `{"error":{"code":"...","message":"..."}}`。invalid_argument=400、not_found=404、conflict/dirty_working_copy/recovery_required=409、busy=423、unsupported=422、内部存储/数据库错误=500。JSON/HTTP 提取器错误可能使用框架自身错误响应。服务会隐藏内部数据库和存储细节，完整诊断在服务端日志。

status 的记录差异在 `diff.changes`，汇总在 `summary`，而不是顶层 changes。diff / conflicts 返回对应数组、total 与 truncated。列表没有游标或 offset；需要完整大结果时，应先扩展分页接口，不能静默接受 truncated。

字段命令自动提交，可提供 author/message。外部 DDL 通过 status/diff 的 `schema_changes` 展示并由 commit 记录；`record_counts_complete: false` 表示结构变化数据集尚未计算行统计。此类 commit 的 `changed_records` 为 null，`rescanned_records` 是全扫描行数，`incremental_changed_records` 是其他数据集的增量数。限制和升级步骤见 [字段结构版本管理](schema-evolution.md)。新字段命令经 gRPC Execute 或 Thrift execute 调用，暂不提供专用类型化 RPC。

## gRPC

协议：`crates/server/proto/geoledger.proto`，包 `geoledger.v1`，服务 `GeoLedger`。提供 Execute、Status、Import、Commit、Log、Diff、Branch、Switch、Merge、Revert、Reset、Restore、Continue、Abort、Recover。

`Switch` 使用 `ReferenceRequest.reference`；`Merge` / `Revert` 使用 `MergeRequest.reference`。响应暂为 JsonReply.json，需再次按 JSON 解码；这是真实 gRPC 传输，但尚不是完全类型化的业务响应模型。

```bash
grpcurl -plaintext -import-path crates/server/proto -proto geoledger.proto \
  -H "Authorization: Bearer $GL_API_TOKEN" \
  -d '{"reference":"HEAD","limit":20}' 127.0.0.1:7879 geoledger.v1.GeoLedger/Log

grpcurl -plaintext -import-path crates/server/proto -proto geoledger.proto \
  -H "Authorization: Bearer $GL_API_TOKEN" \
  -d '{"command_json":"{\"op\":\"branches\"}"}' \
  127.0.0.1:7879 geoledger.v1.GeoLedger/Execute
```

支持 gRPC reflection，当前 reflection 仅暴露公开协议元数据，未绑定业务 token 校验；业务方法均执行鉴权。外部部署需要网关限制 reflection 或关闭它。

## 安全、并发和部署边界

Bearer token 应是至少 24 字节的随机值，HTTP/gRPC 使用同一令牌。仓库路径和数据库连接在服务启动时绑定，调用者不能通过请求指定任意路径或数据库。没有租户隔离和用户权限分级。

请求大小上限 4 MiB，回复序列化上限 16 MiB，同时执行槽位 8。回复在阻塞工作线程中使用有界 writer 编码一次，HTTP/gRPC 复用编码结果；槽位在编码完成后才释放。跨进程仓库文件锁使同仓库操作串行，竞争时可返回 busy；初版不要把并发槽位当成多分支并发写能力。数据库等待表锁上限 5 秒、语句超时默认 120 秒，可用 GL_STATEMENT_TIMEOUT_SECS 配置。初始导入仍可能执行多条语句，并无统一总任务时限。

客户端断开不表示数据库操作被取消。不要自动重试可能已经成功的非幂等写入；先查询状态、历史与 recover。服务尚无请求幂等键，也没有 HTTP OpenAPI 自动生成文件。

## Volo Thrift

IDL：`crates/thrift-gen/idl/geoledger.thrift`，Rust 命名空间 `geoledger.v1`，服务 `GeoLedger`。使用 Volo 0.12.4、volo-thrift 0.12.6、volo-build 0.12.3；构建时需要当前工具链安装 rustfmt（`rustup component add rustfmt`）。版本由 Cargo.lock 固定，建议 `cargo build --workspace --locked`。

启动：`gl --repo ./demo-repo serve-thrift --thrift 127.0.0.1:7880`。它是独立进程入口，Volo 负责 SIGINT/SIGTERM 的连接排空。数据库连接和 SQL 超时沿用 CLI 全局配置。与另一个入口同时操作同一仓库时仍遵守仓库锁。

提供 `execute`、`status`、`import`、`commit`、`log`、`diff`、`branch`、`switch`、`merge`、`revert`、`reset`、`restore`、`continue`、`abort`、`recover`，语义与上述 gRPC 对应。所有方法接受 `(request, optional authorization)`，后者为完整 `Bearer <token>` 字符串。IDL 的 request 字段是 required；空 author 默认 unknown，空 schema 默认 public，空 reference/分支起点默认 HEAD（switch 除外），limit=0 默认 100，负数返回 invalid_argument。

成功返回 `JsonReply.json`，需要 JSON 解码；业务错误通过 IDL 声明的 `ApiError { code, message }` exception 返回，保留 Application 错误码。Volo Rust 客户端需同时处理外层传输错误和 `MaybeException::Exception`，不能只检查 RPC Result。连接失败和协议解析错误由 Volo 返回。

客户端使用 **Framed transport + Binary protocol**，不要使用默认 TTHeader 客户端配置：Rust 示例通过 `DefaultMakeCodec::framed()` 显式选择。其他语言可从此 IDL 生成 Apache Thrift 客户端，使用 TFramedTransport 和 TBinaryProtocol；本轮自动化验证的是 Volo Rust 客户端。读取示例：

```bash
cargo run -p geoledger-server --example thrift_client -- 127.0.0.1:7880
```

四种字段变更通过 `execute(ExecuteRequest { command_json }, authorization)` 传入统一命令协议中的 `alter_schema` JSON。所有方法调用同一个 Service/Application，复用 8 个执行槽位、阻塞任务隔离和 16 MiB JSON 响应上限。解码后的 request 按 Binary 编码大小限制为 4 MiB；帧上限为 16 MiB + 64 KiB，以容纳回复和协议开销。这两个限制含义不同，帧解码仍可能先分配大于 4 MiB 的缓冲区。

### 生成代码与内存安全边界

`crates/thrift-gen` 构建时从 IDL 生成绑定，依赖的 Volo 生成器会产生含 unsafe 的多服务路由实现。因此只有该生成 crate 不继承 workspace 的 `unsafe_code = forbid`；手写代码继续禁止 unsafe。当前服务器使用类型化 `GeoLedgerServer::new`，不使用字节路由器，也未启用 Volo 的 `unsafe-codec` / `unsafe_unchecked` 特性。生成代码不手工修改。升级生成器时应重新检查这条边界和 MSRV；当前锁文件把间接依赖 ordered-float 保持在支持 Rust 1.88 的 5.1.0。

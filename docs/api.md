# GeoLedger API v1

## Rust 与统一命令

依赖 [geoledger 应用包](../crates/app/Cargo.toml)，通过 `Application::new(path)`、`with_provider(Arc<dyn WorkingCopyProvider>)` 和 `execute(Command)` 调用，返回 `Result<serde_json::Value, geoledger_core::Error>`。同步调用在当前线程执行，异步宿主使用 `spawn_blocking`。示例见 [library.rs](../crates/app/examples/library.rs)。

HTTP `POST /v1/commands`、gRPC `Execute` 和 Thrift `execute` 接收同一 Command JSON。完整字段定义见 [command.rs](../crates/app/src/command.rs)。作者省略时统一为 `mapseekai`；类型化 gRPC / Thrift 的空作者同样使用该值。显式作者优先。schema 默认 `public`，查询起点默认 `HEAD`，limit 默认 100、最多 1000 项。请求字段按声明严格校验。

以下每行是一个独立请求：

```json
{"op":"init","author":"mapseekai"}
{"op":"import","dataset":"roads","schema":"public","table":"roads"}
{"op":"status","limit":100}
{"op":"diff","from":"main","to":"draft","limit":100}
{"op":"commit","message":"更新道路"}
{"op":"log","reference":"HEAD","limit":100}
{"op":"show","reference":"HEAD","dataset":"roads","key":"1001"}
{"op":"schema","dataset":"roads","reference":"HEAD"}
{"op":"alter_schema","dataset":"roads","change":{"action":"add","name":"note","data_type":"text"}}
{"op":"alter_schema","dataset":"roads","change":{"action":"rename","name":"note","new_name":"memo"}}
{"op":"alter_schema","dataset":"roads","change":{"action":"alter_type","name":"memo","data_type":"varchar(200)"}}
{"op":"alter_schema","dataset":"roads","change":{"action":"drop","name":"memo","discard":true}}
```

```json
{"op":"branches"}
{"op":"branch","name":"draft","from":"HEAD"}
{"op":"switch","branch":"draft"}
{"op":"merge","source":"draft"}
{"op":"conflicts","limit":100}
{"op":"resolve","dataset":"roads","key":"1001","choice":"theirs"}
{"op":"merge_continue"}
{"op":"merge_abort"}
{"op":"restore","discard":true}
{"op":"reset","target":"HEAD","hard":true}
{"op":"revert","target":"FULL_COMMIT_ID"}
{"op":"recover"}
{"op":"fsck"}
{"op":"reflog","limit":100}
```

`FULL_COMMIT_ID` 替换为 log 返回的完整提交 ID。diff 省略 to 时比较工作副本；show 的 dataset 与 key 成对提供，省略时查看提交。merge 返回值同时表达提交或冲突状态，客户端按业务结果进入后续流程。

自定义解决使用 `choice: "custom"` 和完整 `record`，例如 `{"key":"1001","fields":{"id":{"type":"text","value":"1001"},"name":{"type":"text","value":"road"},"geom":{"type":"null"}}}`。提供 schema 的全部字段，主键字段值与 key 一致，NULL 符合可空性定义。几何使用 `type: "geometry"` 和小写 XDR EWKB 十六进制 value，由适配器进行类型校验及规范化。

status 的差异位于 `diff.changes`，汇总位于 `summary`。diff / conflicts 返回结果数组、total 与 truncated。status/diff 预览采用约 8 MiB 载荷预算，单条大记录可超过预算；输出条数由 limit 和实际载荷共同决定。结构变化通过 `schema_changes` 与 `record_counts_complete` 表达，提交返回 rescanned_records 等统计，详见 [字段结构版本管理](schema-evolution.md)。

## HTTP 路由

| 路由 | 功能 |
|---|---|
| GET /health | 服务版本与存活 |
| POST /v1/commands | 全部统一命令 |
| GET /v1/status、GET /v1/log | 状态和历史，可携带 limit 等查询参数 |
| GET /v1/branches、POST /v1/branches | 查询或创建分支 |
| GET /v1/conflicts | 查询合并冲突 |
| POST /v1/commits、POST /v1/merges | 提交或合并，省略 author 时使用 mapseekai |

路由定义见 [http.rs](../crates/server/src/http.rs)。错误以 `error.code` 和 `error.message` 表达：invalid_argument 对应 400，not_found 对应 404，conflict / dirty_working_copy / recovery_required 对应 409，busy 对应 423，unsupported 对应 422，后端错误对应 500。鉴权错误为 401。完整内部诊断保存在服务端日志。

## gRPC

[geoledger.proto](../crates/server/proto/geoledger.proto) 定义 `geoledger.v1.GeoLedger`，提供 Execute、Status、Import、Commit、Log、Diff、Branch、Switch、Merge、Revert、Reset、Restore、Continue、Abort 和 Recover。Switch 使用 reference；Merge / Revert 使用 MergeRequest.reference。响应通过 `JsonReply.json` 表达动态数据，客户端按 JSON 解码。

从项目根目录启动 `gl --repo ./demo-repo serve --http 127.0.0.1:7878 --grpc 127.0.0.1:7879`，在服务运行时从另一个终端调用：

```bash
grpcurl -plaintext -import-path crates/server/proto -proto geoledger.proto \
  -H "Authorization: Bearer $GL_API_TOKEN" \
  -d '{"reference":"HEAD","limit":20}' 127.0.0.1:7879 geoledger.v1.GeoLedger/Log
```

地址由上述启动命令创建；两个终端配置相同的 GL_API_TOKEN。Rust 客户端见 [client.rs](../crates/server/examples/client.rs)。gRPC reflection 提供公开协议元数据，业务方法执行令牌鉴权，外部部署通过网关控制 reflection 的访问范围。

## Volo Thrift

[geoledger.thrift](../crates/thrift-gen/idl/geoledger.thrift) 定义 `geoledger.v1.GeoLedger`，提供与 gRPC 对等的 15 个方法。启动命令为 `gl --repo ./demo-repo serve-thrift --thrift 127.0.0.1:7880`，该进程创建对应的本地监听地址。客户端示例见 [thrift_client.rs](../crates/server/examples/thrift_client.rs)，服务运行时执行 `cargo run -p geoledger-server --example thrift_client -- 127.0.0.1:7880`。

客户端采用 **Framed transport + Binary protocol**，Rust 示例通过 `DefaultMakeCodec::framed()` 选择协议。所有方法接受 request 与 optional authorization，后者使用完整 `Bearer <token>`。空作者默认 mapseekai，空 schema 默认 public，查询起点默认 HEAD；limit 为 0 时采用 100，负数返回 invalid_argument。Switch 的 reference 显式指定目标分支。

成功响应为 JsonReply.json，业务错误通过 ApiError exception 返回。客户端同时处理外层传输结果及 MaybeException 业务结果。四种字段操作通过 execute 传入 alter_schema JSON，全部方法复用 Service / Application。

## 安全、并发与容量

Bearer token 使用至少 24 字节的随机值，授权范围为绑定仓库的读写及恢复操作。仓库路径和数据库连接在服务启动时确定。服务默认使用 loopback，跨机器部署通过 TLS 反向代理保护传输，并在部署层配置访问隔离。

请求容量为 4 MiB，JSON 响应容量为 16 MiB，同时执行槽位为 8。Thrift 帧容量为 16 MiB + 64 KiB，解码后按请求 Binary 编码大小执行 4 MiB 检查，因此帧解码可以使用更大的缓冲区。响应在阻塞工作线程中有界编码，编码期间持续持有执行槽位。跨进程仓库锁串行化同一仓库的操作。

数据库表锁等待为 5 秒，每条事务内 SQL 默认超时 120 秒，GL_STATEMENT_TIMEOUT_SECS 可调整。一个命令可以包含多条 SQL。客户端断连后的操作状态通过 status、log 和 recover 核查，再决定是否重新发起写入。

## 生成代码与内存安全

[thrift-gen](../crates/thrift-gen) 在构建时由 IDL 生成绑定。Volo 生成器产生包含 unsafe 的路由代码，因此该生成 crate 单独管理其构建安全约束；手写 workspace 代码使用 unsafe_code = forbid。服务器使用类型化 GeoLedgerServer，绑定通过构建脚本维护。

Cargo.lock 固定 Volo 及相关依赖，构建器使用 rustfmt。生成器升级时一并检查生成代码和 Rust 工具链要求。功能验证见 [验证记录](verification.md)。

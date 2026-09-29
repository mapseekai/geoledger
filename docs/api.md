# API

[项目概览](../README.md) · [快速开始](getting-started.md) · [功能指南](user-guide.md) · [开发说明](development.md)

## 统一命令

Rust、HTTP、gRPC 和 Thrift 共用 Application。HTTP `POST /v1/commands`、gRPC `Execute`、Thrift `execute` 接收相同 JSON。以下每行是一个独立请求，完整字段见 [Command 定义](../crates/app/src/command.rs)：

```json
{"op":"init"}
{"op":"import","dataset":"roads","schema":"geoledger_demo","table":"roads"}
{"op":"status","limit":100}
{"op":"commit","message":"更新道路"}
{"op":"log","reference":"HEAD","limit":20}
{"op":"alter_schema","dataset":"roads","change":{"action":"add","name":"note","data_type":"text"}}
{"op":"merge","source":"draft"}
{"op":"resolve","dataset":"roads","key":"1","choice":"theirs"}
{"op":"fsck"}
```

记录以主键 `key` 和字段映射 `fields` 表达。属性使用数据库规范文本，几何使用小写 XDR EWKB；自定义冲突结果提供符合 schema 的完整记录。合并响应包含提交或冲突状态，客户端据此继续处理。

## HTTP

| 路由 | 功能 |
|---|---|
| `GET /health` | 服务版本与存活 |
| `POST /v1/commands` | 执行统一命令 |
| `GET /v1/status`、`GET /v1/log` | 查询状态与历史 |
| `GET /v1/branches`、`POST /v1/branches` | 查询或创建分支 |
| `GET /v1/conflicts` | 查询记录冲突 |
| `POST /v1/commits`、`POST /v1/merges` | 提交或合并 |

下面在 PowerShell 生成令牌并启动服务，监听地址由该命令创建：

```powershell
$bytes = New-Object byte[] 32
$rng = [System.Security.Cryptography.RandomNumberGenerator]::Create()
$rng.GetBytes($bytes)
$rng.Dispose()
$env:GL_API_TOKEN = [Convert]::ToBase64String($bytes)
.\gl.exe --repo '.\demo-repo' serve --http 127.0.0.1:7878 --grpc 127.0.0.1:7879
```

沿用快速开始的 `GL_DATABASE_URL`。服务运行时，在另一终端输入相同令牌读取状态；服务终端通过 Ctrl+C 结束：

```powershell
$token = Read-Host '输入服务端配置的 GL_API_TOKEN'
Invoke-RestMethod -Headers @{Authorization="Bearer $token"} -Uri 'http://127.0.0.1:7878/v1/status'
```

## gRPC 与 Thrift

| 接口 | 定义与调用方式 |
|---|---|
| gRPC | [geoledger.proto](../crates/server/proto/geoledger.proto) 定义 `geoledger.v1.GeoLedger`，通过 HTTP/2 调用，令牌放入 Authorization metadata |
| Volo Thrift | [geoledger.thrift](../crates/thrift-gen/idl/geoledger.thrift) 定义 `GeoLedger`，采用 Framed Binary，令牌通过 authorization 参数传入 |

两种接口提供执行、状态、导入、提交、历史、差异、分支、切换、合并、撤销、重置、恢复工作副本、继续、取消与恢复中断操作共 15 个方法。响应使用 `JsonReply.json`，客户端按 JSON 解码；字段演进通过 Execute / execute 调用统一命令。

Unix 构建通过 `gl --repo ./demo-repo serve-thrift --thrift 127.0.0.1:7880` 启动 Thrift。客户端示例见 [gRPC](../crates/server/examples/client.rs) 和 [Thrift](../crates/server/examples/thrift_client.rs)。

## Rust

使用 `Application::new(path)` 创建入口，`with_provider` 绑定工作副本，`execute(Command)` 执行操作。返回值为 `Result<serde_json::Value, geoledger_core::Error>`。异步宿主通过 `spawn_blocking` 调用同步应用层，示例见 [library.rs](../crates/app/examples/library.rs)。

## 配置与响应

`GL_DATABASE_URL` 配置数据库连接；`--database-env` 可指定连接变量名称。`GL_STATEMENT_TIMEOUT_SECS` 或 `--statement-timeout-secs` 配置每条 SQL 的超时，默认 120 秒，范围 1–2147483 秒。

服务使用至少 24 字节的随机 Bearer token，授权范围为绑定仓库的读写及恢复操作。跨机器部署使用 TLS 反向代理；gRPC reflection 提供协议元数据，通过网关管理其访问范围。

请求上限为 4 MiB，JSON 响应上限为 16 MiB。列表最多返回 1000 项，预览同时采用约 8 MiB 载荷预算，单条大记录可超过该预算。读取 `total`、`truncated` 判断返回范围；结构变化使用 `schema_changes`、`record_counts_complete` 表达，提交通过 `rescanned_records` 返回整表扫描数量。

HTTP 应用错误返回 `error.code` 与 `error.message`：参数校验 400、鉴权 401、资源查找 404、状态冲突 409、仓库占用 423、能力校验 422、后端异常 500。Thrift 通过 `ApiError` 表达业务结果；客户端同时检查传输状态。连接中断后先通过 `status`、`log`、`recover` 核对结果，再决定是否重新发起写入。

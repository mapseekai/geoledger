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

HTTP 输入解析与应用错误统一返回 `error.code`、`error.message` 和 `error.request_id`：参数校验 400、鉴权 401、资源查找 404、状态冲突 409、仓库占用 423、能力校验 422、后端异常 500。Thrift 通过 `ApiError` 表达业务结果；客户端同时检查传输状态。连接中断后先通过 `status`、`log`、`recover` 核对结果，再决定是否重新发起写入。

## 中心版

中心 HTTP 操作均为 `POST /api/center/{operation}`，JSON 请求体，`Authorization: Bearer TOKEN`。
所有操作进入独立的 `CenterApplication`。身份只由服务令牌映射确定；额外的作者字段会被拒绝。
接口返回 JSON，错误统一为 `error.code`、`error.message`、`error.request_id`，冲突详情保留在响应顶层。状态使用 400（参数）、401（认证）、404（缺失或无权访问）、409（版本/合并/幂等冲突）、
413（大小限制）、422（合并结果校验）、429（繁忙）、503（数据库操作失败，隐藏连接详情）或 504（操作截止时间）。

| operation | 请求字段（? 表示可选） |
|---|---|
| create_project / list_projects | `name` / `after?, limit?` |
| get_project | `project` |
| set_member | `project, subject, role` |
| create_dataset / list_datasets | `project, name` / `project, after?, limit?` |
| create_workspace / list_workspaces | `project` / `project, after?, limit?` |
| get_workspace | `project, workspace` |
| save | `project, workspace, expected_workspace_version, edits` |
| discard | `project, workspace, expected_workspace_version` |
| features | `project, dataset, workspace?, revision?, feature_id?, bbox?, after?, limit?` |
| diff / conflicts | `project, workspace, after?, limit?` |
| history | `project, after?, limit?` |
| commit | `project, revision, after?, limit?` |
| publish | `project, workspace, expected_workspace_version, request_id, message` |
| resolve / rebase | `project, workspace, expected_workspace_version, expected_head, resolutions` |
| restore | `project, revision` |

`edits` / `resolutions` 元素为 `{dataset, feature_id, feature}`，`feature` 必须显式提供，为完整 GeoJSON Feature 或删除标记 null。
Feature 必含 `type: "Feature"`、与 `feature_id` 相等的字符串 `id`、对象 `properties`、`geometry`。
`features` 返回 FeatureCollection；传 `feature_id` 返回单个 Feature。两者携带 `revision` 和可空的 `workspace_version`，用于下一次乐观保存。`workspace` 与 `revision` 二选一，均省略时读取已发布 HEAD。
几何采用 EPSG:4326 的 XY / XYZ 坐标，支持 GeoJSON 几何及 null，属性保留 JSON 类型。
`bbox` 为 `[west,south,east,north]`。每页默认 100、最大 1000；继续查询使用最后一项 ID，
历史列表使用 revision，diff/commit 使用返回的 `cursor`，FeatureCollection 也提供 `next_after`；空页结束。
正式数据分页续读时携带首屏返回的 `revision`，获得固定版本结果；草稿续页同时核对返回的 `workspace_version`，
版本变化时重新读取草稿。
请求及响应限 4 MiB，同时最多 16 个请求；响应预览还按展开后的数据结构计入内存预算。Feature 输入限 16 KiB、256 个属性，属性名/要素 ID 限 256 字节，
项目/集合名限 256 字节，subject 限 128 字节，提交消息限 2048 字节。文本标识非空且无控制字符。
普通查询响应超限时减小 `limit`；冲突响应提供 `total`、`truncated` 和 `next_after`，通过 `conflicts` 继续读取。
已解决选择在正式 HEAD 变化或后续草稿保存后返回 `reason: "stale_resolution"`，保留选择内容并要求再次确认；
即使选择删除或恢复原值，也保留该确认流程。
`resolve` 每批最多处理 100 个冲突；`rebase` 一次处理全部剩余冲突并更新基线。两者均核对 HEAD 与工作区版本。命名与 ID 大小均按 UTF-8 字节计算。

在[启动示例](getting-started.md#中心版)生成令牌文件并运行服务后，另一个终端可使用 curl 创建资源。令牌文件路径使用启动服务时的实际文件：

```sh
export GL_CENTER_TOKEN_FILE="$PWD/.center-tokens.json"
TOKEN=$(python3 -c 'import json,os; print(json.load(open(os.environ["GL_CENTER_TOKEN_FILE"]))[0]["token"])')
api() { curl --fail-with-body -sS "http://127.0.0.1:7881/api/center/$1" -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' --data-binary "$2"; }
field() { python3 -c 'import json,sys; print(json.load(sys.stdin)[sys.argv[1]])' "$1"; }
PROJECT=$(api create_project '{"name":"browser-demo"}' | field project)
DATASET=$(api create_dataset "{\"project\":\"$PROJECT\",\"name\":\"places\"}" | field dataset)
WORKSPACE=$(api create_workspace "{\"project\":\"$PROJECT\"}" | field workspace)
api save "{\"project\":\"$PROJECT\",\"workspace\":\"$WORKSPACE\",\"expected_workspace_version\":0,\"edits\":[{\"dataset\":\"$DATASET\",\"feature_id\":\"place-1\",\"feature\":{\"type\":\"Feature\",\"id\":\"place-1\",\"properties\":{\"name\":\"Park\",\"open\":true},\"geometry\":{\"type\":\"Point\",\"coordinates\":[116.4,39.9]}}}]}"
REQUEST_ID=$(python3 -c 'import uuid; print(uuid.uuid4())')
api publish "{\"project\":\"$PROJECT\",\"workspace\":\"$WORKSPACE\",\"expected_workspace_version\":1,\"request_id\":\"$REQUEST_ID\",\"message\":\"Seed places\"}"
api features "{\"project\":\"$PROJECT\",\"dataset\":\"$DATASET\"}"
unset TOKEN
```

PowerShell 7 示例（同一服务，创建另一组资源）：

```powershell
$entry = @(Get-Content -Raw '.center-tokens.json' | ConvertFrom-Json)[0]
$headers = @{Authorization="Bearer $($entry.token)"}
function Center($op, $body) {
  Invoke-RestMethod -Method Post -Uri "http://127.0.0.1:7881/api/center/$op" -Headers $headers -ContentType 'application/json' -Body (ConvertTo-Json -InputObject $body -Depth 20 -Compress)
}
$p = (Center 'create_project' @{name='powershell-demo'}).project
$d = (Center 'create_dataset' @{project=$p; name='places'}).dataset
$w = (Center 'create_workspace' @{project=$p}).workspace
$feature = @{type='Feature'; id='place-1'; properties=@{name='Park'; open=$true}; geometry=@{type='Point'; coordinates=@(116.4,39.9)}}
Center 'save' @{project=$p; workspace=$w; expected_workspace_version=0; edits=@(@{dataset=$d; feature_id='place-1'; feature=$feature})}
$request = @{project=$p; workspace=$w; expected_workspace_version=1; request_id=[guid]::NewGuid().ToString(); message='Seed places'}
Center 'publish' $request
Center 'features' @{project=$p; dataset=$d}
Remove-Variable entry,headers
```

中心数值协议按 i64/u64 范围精确保存整数，也识别表示整数的指数和小数写法；非整数采用有限 binary64，并在写入前校验范围。任意精度十进制值可使用字符串属性。浏览器测试台原样发送请求文本并显示响应文本，保留大整数字面量。

`GL_CENTER_OPERATION_TIMEOUT_MS` 配置中心操作的总截止时间，默认 30000 毫秒，范围 1–300000 毫秒；超时后的发布使用原 `request_id` 和相同请求体重试确认结果。Thrift 入口按严格 Framed Binary 校验帧、字段长度、UTF-8 和嵌套深度，单个入站帧上限为 4 MiB + 64 KiB；每个监听器最多接纳 8 个连接，帧读取截止时间为 10 秒、响应写入为 30 秒。客户端空闲后可重新建立连接。

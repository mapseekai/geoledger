# API 与 SDK

[项目概览](../README.md) · [使用指南](user-guide.md) · [存储扩展](storage.md)

所有示例从仓库根目录准备依赖，并连接已经运行的业务服务。地址、用户凭证和开发服务的准备方式见 [快速开始](getting-started.md)。SDK 示例中的写操作会创建数据，使用专用测试服务。

## SDK 的公开接口

Go、Rust、TypeScript / Node.js、Python 提供普通业务方法和工作区对象。
用户配置服务地址与访问令牌，即可通过业务方法完成资源管理、编辑与发布；SDK 封装协议请求、认证和业务错误。
内部统一使用 tonic 服务的 gRPC 协议，浏览器控制台使用 HTTP。

典型流程：创建项目 → 创建数据集 → 创建工作区 → 保存 GeoJSON → 发布。
工作区对象维护草稿版本，发布时生成并保留请求 ID。修改失败时保留原版本；
先查询冲突或刷新工作区，再决定是否重做修改。写操作和重试由调用方明确发起。

同一工作区对象的修改由语言自身的机制协调：Rust 使用可变借用，Go 使用互斥锁，
Python 使用 RLock，TypeScript 进行同句柄并发校验。多个独立对象的并发编辑由服务端乐观版本检查协调。

默认 SDK 地址 `http://127.0.0.1:7882`，控制台 HTTP 地址使用另一个端口。
SDK 支持 `https://host:port` 并校验服务端证书；默认超时 30 秒，不设应用层固定消息字节上限。

`http://` 地址只用于回环主机（`127.0.0.1`、`::1`、`localhost`）。确需在隔离内网使用明文时，设置环境变量 `GL_ALLOW_INSECURE_TRANSPORT=true` 或使用各语言的显式选项：

| 语言 | 明文选项 | 私有 CA |
| --- | --- | --- |
| Go | `DialWithOptions(endpoint, token, DialOptions{AllowInsecure: true})` | 系统信任库 |
| Python | `Client(endpoint, token, allow_insecure=True)` | `GRPC_DEFAULT_SSL_ROOTS_FILE_PATH` |
| Rust | `Client::connect_with(endpoint, token, ConnectOptions { allow_insecure: true, ..Default::default() })` | `ConnectOptions::ca_pem` |
| TypeScript | `new Client(endpoint, token, timeoutMs, { allowInsecure: true })` | `NODE_EXTRA_CA_CERTS` 或系统信任库 |
| CLI | `gl --allow-insecure` | `gl --ca-file ca.pem` |

## Python

```sh
python -m pip install ./sdk/python
```

```python
import os
from geoledger import Client

with Client("http://127.0.0.1:7882", os.environ["GL_TOKEN"]) as client:
    project = client.create_project("城市道路")
    dataset = client.create_dataset(project.id, "roads", "point")
    draft = client.create_workspace(project.id)
    draft.save(dataset.id, {
        "type": "Feature", "id": "road-1",
        "properties": {"name": "滨江大道"},
        "geometry": {"type": "Point", "coordinates": [120, 30]},
    })
    receipt = draft.publish("新增道路")
    rows = client.features(project.id, dataset.id, revision=receipt["revision"])
    print(receipt["revision"], rows)
```

在当前环境设置 `GL_TOKEN` 后运行此示例，输出发布修订及该修订的要素。业务资源使用不可变数据类；要素、差异和审计使用普通字典。完整流程见
[Python 示例](../sdk/python/tests/smoke.py)。

## Rust

客户端 crate 为 [geoledger-client](../sdk/rust/Cargo.toml)，位于同一 Cargo 工作区。以下代码放在 Tokio 异步函数中，`endpoint` 和 `token` 为应用配置的服务地址与令牌。

```rust,ignore
use geoledger_client::Client;
use serde_json::json;

let client = Client::connect(endpoint, &token).await?;
let project = client.create_project("城市道路").await?;
let dataset = client.create_dataset(&project.id, "roads", "point").await?;
let mut draft = client.create_workspace(&project.id).await?;
draft.save(&dataset.id, json!({
    "type":"Feature", "id":"road-1",
    "properties":{"name":"滨江大道"}, "geometry":null
})).await?;
let receipt = draft.publish("新增道路").await?;
```

结果是 SDK 自身的业务结构，GeoJSON 使用 `serde_json::Value`。
客户端可克隆复用连接，工作区修改使用 `&mut self`。
完整示例见 [Rust 示例](../sdk/rust/examples/smoke.rs)。

## Go

模块为 [github.com/mapseekai/geoledger/sdk/go](../sdk/go/go.mod)，要求 Go 1.25+。以下片段位于返回 `error` 的函数内，导入该模块为 `geoledger`，使用已配置的 `endpoint`、`token` 和 `context.Context` 类型的 `ctx`。所有网络调用接受标准 context。

```go
client, err := geoledger.Dial(endpoint, token)
if err != nil { return err }
defer client.Close()
project, err := client.CreateProject(ctx, "城市道路")
if err != nil { return err }
dataset, err := client.CreateDataset(ctx, project.ID, "roads", "point")
if err != nil { return err }
draft, err := client.CreateWorkspace(ctx, project.ID)
if err != nil { return err }
_, err = draft.Save(ctx, dataset.ID, geoledger.Feature{
    Type: "Feature", ID: "road-1",
    Properties: map[string]any{"name": "滨江大道"}, Geometry: nil,
})
if err != nil { return err }
receipt, err := draft.Publish(ctx, "新增道路")
```

也可传入普通 JSON 结构或 `json.RawMessage`。读取的 GeoJSON 使用 `json.RawMessage`，
可解码到自己的结构；解码到通用 map 时使用 `json.Decoder.UseNumber()` 保持大整数。
完整示例见 [Go 示例](../sdk/go/cmd/smoke/main.go)。

## TypeScript / Node.js

```sh
npm ci --prefix sdk/ts
npm run build --prefix sdk/ts
```

```typescript
import { Client } from "@geoledger/client";
const endpoint = process.env.GL_ENDPOINT ?? "http://127.0.0.1:7882";
const token = process.env.GL_TOKEN!;
const client = new Client(endpoint, token);
try {
  const project = await client.createProject("城市道路");
  const dataset = await client.createDataset(project.id, "roads", "point");
  const draft = await client.createWorkspace(project.id);
  await draft.save(dataset.id, {
    type: "Feature", id: "road-1",
    properties: { name: "滨江大道", exact: 9007199254740993n },
    geometry: null,
  });
  const receipt = await draft.publish("新增道路");
  const rows = await client.features(project.id, dataset.id, {revision: receipt.revision});
} finally { client.close(); }
```

所有调用返回 Promise。版本与 64 位计数使用 `bigint`，普通 GeoJSON 属性中的安全数字保持 `number`，
大整数读为 `bigint`。`parseJson` / `stringifyJson` 用于精确 JSON 读写，保持大整数的精确表示。
也可直接传原始 GeoJSON 文本。对象中的大整数使用 `bigint`，也可通过原始文本保存精确数值。
完整示例见 [TypeScript 示例](../sdk/ts/test/smoke.cjs)。此 SDK 面向 Node.js。

## 运行完整示例

四个语言的可执行示例从私有 `GL_TOKEN_FILE` 读取测试身份，连接 `GL_ENDPOINT` 指定的服务。设置专用测试服务地址和凭证路径后执行：

```sh
cargo run --locked -p geoledger-client --example smoke
python sdk/python/tests/smoke.py
node sdk/ts/test/smoke.cjs
go -C sdk/go run ./cmd/smoke
```

Python SDK 与 TS SDK 分别按上文安装和构建；统一安装与联调流程见 [四语言联调](development.md#四语言联调)。成功时，各示例输出完成提示，业务流程包含创建、精确要素保存、发布、重试和读取。

## 常用操作

| 对象 | 业务方法（Python / Rust 命名；Go / TS 使用各自命名惯例） |
|---|---|
| Client | info、create_project、project、projects、set_member |
| Client | members、remove_member、archive_project、delete_project |
| Client | create_dataset、datasets、create_workspace、workspace、workspaces |
| Client | features、history、commit、audit、restore |
| Workspace | save、save_batch、delete、features、diff、conflicts |
| Workspace | publish、resolve、rebase、discard、refresh |

`save` 直接接收带字符串 `id` 的完整 GeoJSON Feature。批量编辑使用 SDK 自身的 `Edit`，
删除通过 `delete()` 或显式 `null` / `None` 表达，保存操作提供完整要素字段。
正式要素分页固定首次返回的 revision，后续查询带该版本及 next_after；草稿分页需核对 workspace_version。
`history` 的每条提交还包含实际发布工作区及其发布时基线：Go 使用 `SourceWorkspace` / `SourceBaseRevision`，Rust/Python 使用 `source_workspace` / `source_base_revision`，TypeScript 使用 `sourceWorkspace` / `sourceBaseRevision`。基线随发布前 rebase 更新；已发布工作区保持其基线不变。
JSON 对象键通过递归校验，保护普通对象的编解码语义；
`$serde_json::private::RawValue` 和 `$serde_json::private::Number` 为保留键，
包含嵌套或转义写法的输入会获得 `invalid_argument` 校验结果。普通字符串值可以包含这些文字。
小数按 binary64 舍入，编解码保持已舍入的值。

## 错误与发布恢复

Python / TS 捕获 `GeoLedgerError`；Rust 使用 `Error`；Go 用 `errors.As` 获取 `*geoledger.Error`。
它们提供业务 `code`、message、request_id / requestId、可选 conflicts，以及 `uncertain` / Uncertain。
常见业务码包括 `invalid_argument`、`unauthenticated`、`not_found`、`conflict`、`timeout` 和 `unavailable`。
调用方通过 SDK 的业务错误类型处理结果。

当 `uncertain` 为真时，写入可能已经完成。对同一工作区再次 `publish` 并使用原说明即可重试原请求；
期间工作区对象保持原要素上下文和发布说明，以便确认原请求。成功后再次调用也返回原发布结果。
发布被明确拒绝时释放待重试请求，保留草稿版本供查询、解决冲突。
已有待确认请求在重试遇到身份验证失败、权限拒绝或资源不可见时仍然保留；
先前发布的结果通过原请求确认。恢复访问后继续使用原请求及其请求 ID。

工作区的 `pending_publication` / `PendingPublication()` / `pendingPublication` 暴露可持久化的业务发布意图，
可在跨进程恢复时交给 `client.publish(intent)`。Python 使用 `dataclasses.asdict`，Rust / Go 使用 JSON 序列化，
TS 使用 `stringifyJson`；恢复时用 `parseJson` 读取，并将 `expectedWorkspaceVersion` 转为 `BigInt`，以恢复版本字段的类型。句柄在客户端内存中维护版本与请求状态，业务数据和历史由服务端持久化。
同一工作区的多个独立句柄仍受服务端乐观版本检查约束，并发修改通过显式刷新和冲突处理协调。

大量冲突时错误只含 head、version、total 等摘要，完整内容通过工作区 `conflicts()` 分页查询。

## HTTP 业务 API

`POST /api/v1/{operation}`，使用 snake_case 操作名，例如 `create_project`、`save`、`publish`、`features`。JSON 请求体字段对应 RPC 请求；HTTP 的 `feature` 为 GeoJSON 对象。列表响应为 JSON 数组，Features 返回 FeatureCollection 或单个 Feature。

```sh
curl --fail-with-body http://127.0.0.1:7881/api/v1/create_project \
  -H "Authorization: Bearer $GL_TOKEN" -H 'Content-Type: application/json' \
  --data '{"name":"roads"}'
```

Web 控制台通过 Next.js 的 `/api/session` 管理登录和退出，通过 `/api/console` 调用经过校验的业务方法。应用直接使用 HTTP 时调用上面的 GeoLedger 服务端 API，使用自己的 Bearer 令牌。两条路径由同一业务应用层执行权限和事务语义。

业务服务 `GET /` 为 JSON 服务信息；管理界面由独立 Web 项目提供。`GET /health` 为存活检查；`GET /ready` 最多 2 秒检查存储；`GET /metrics` 要求同样的 Bearer 认证。默认同源，跨域授权需由部署网关明确配置。

## 服务端协议与请求规格

[geoledger.proto](../proto/geoledger/v1/geoledger.proto) 是内部统一协议定义。
生成代码位于 SDK 的私有 / internal 目录，公开入口仅提供业务接口。
HTTP 与 RPC 仍调用同一个 Application；用户身份只来自服务端验证的令牌。
SDK 可从本仓库源码构建和安装，包注册表分发通过独立发布流程完成。

| 情况 | HTTP |
|---|---|
| 参数或几何非法 | 400 / 422 |
| 身份验证失败 | 401 |
| 项目创建策略或配额拒绝 | 403 |
| 资源不存在或无权查看 | 404 |
| 重复名称、版本、合并或幂等冲突；写入已归档项目；移除最后一名 owner | 409 |
| 大小超限 / 执行容量耗尽 | 413 / 429 |
| 期限耗尽 | 504 |
| 存储不可用 | 503 |

Audit 仅项目 owner 可读。项目与列表结果包含 `state`（active / archived）和调用者的 `role`；成员与生命周期规则见 [项目与成员治理](production.md#项目与成员治理)。每页 1–1000 条、Feature 不设独立字节上限、最多 256 个属性；EPSG:4326，XY/XYZ。
标识采用有效文本字符，属性键和值使用 U+0000 以外的 JSON 文本；服务统一校验输入并返回业务错误。容量配置见 [生产运行](production.md)。

## 当前版本契约

服务端、CLI、四语言 SDK 和 Web 控制台按同一发布版本部署，协议以 [geoledger.proto](../proto/geoledger/v1/geoledger.proto) 为统一来源。CI 检查生成代码的可复现性，并通过真实服务验证各 SDK。

当前 gRPC 包为 `geoledger.v1`，HTTP 入口为 `/api/v1`。接口及存储格式变化在 [CHANGELOG](../CHANGELOG.md) 中说明，调用方随版本一起更新。

当前存储格式为 7，新库直接创建当前结构。同格式的数据备份、恢复和跨后端搬迁见 [备份与恢复](production.md#备份与恢复)。

`Info.max_feature_bytes` 为 `0` 表示不设独立的单要素字节上限；`max_request_bytes` 为 `0` 也表示不设应用层固定请求字节上限。

### 大数据流式传输

`SaveStream` 接收分块编码的单个 `SaveRequest`，`FeaturesStream` 返回分块编码的单个 `FeaturesReply`。每个流首块携带 `total_bytes`，后续块为 0；总长度不符或流中断时不接受不完整数据。服务端收齐保存请求后通过 Application 原子校验与写入，读取结果来自同一快照。Node.js SDK（含 Web BFF）对超过 64 KiB 的保存请求自动使用流式上传，要素查询使用流式下载，每块目标大小 64 KiB，并保留认证、期限和背压。其他 SDK 的普通 RPC 也已取消 4 MiB 固定上限。

当前业务 API 仍在内存中组装完整请求/结果；流式传输解决消息分块，不代表恒定内存占用。实际容量受可用内存、运行时和 gRPC 协议边界影响。发布请求的原始内容、幂等重试与事务语义不变。

创建数据集必须传 `geometry_type`（SDK TypeScript 使用 `geometryType`）：`point`、`line` 或 `polygon`。列表与创建结果返回该字段。新增 `RenameProject`、`RenameDataset`、`DeleteDataset` RPC，以及同名 snake_case HTTP 操作。重命名请求包含 `project`、`name`，数据集操作还包含 `dataset`。删除数据集包含 `project`、`dataset`、`confirm_name`，确认名称必须完全匹配。项目重命名和项目／数据集删除限 owner 或平台管理员；数据集重命名限可写成员。

Save/SaveStream 返回 `warnings` 字符串列表，指出几何自相交等拓扑问题，并在同一保存事务的审计事件中记录。此类几何按原坐标保存，不自动修复；坐标结构、维度、闭环、有限数值、经纬度范围及数据集几何类型仍严格校验。工作区累计要素数不再限制为 1000，单次 Save 仍限制 1–100 个修改。

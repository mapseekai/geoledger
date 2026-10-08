# API 与 SDK

[项目概览](../README.md) · [使用指南](user-guide.md) · [存储扩展](storage.md)

## SDK 的公开接口

Go、Rust、TypeScript / Node.js、Python 提供普通业务方法和工作区对象。
用户配置服务地址与访问令牌，即可通过业务方法完成资源管理、编辑与发布；SDK 封装协议请求、认证和业务错误。
内部统一使用 tonic 服务的 gRPC 协议，浏览器控制台使用 HTTP。

典型流程：创建项目 → 创建数据集 → 创建工作区 → 保存 GeoJSON → 发布。
工作区对象维护草稿版本，发布时生成并保留请求 ID。修改失败时保留原版本；
先查询冲突或刷新工作区，再决定是否重做修改。写操作和重试由调用方明确发起。

默认 SDK 地址 `http://127.0.0.1:7882`，控制台 HTTP 地址使用另一个端口。
SDK 支持 `https://host:port` 并校验服务端证书；默认超时 30 秒，消息上限 4 MiB。

## Python

```sh
python -m pip install ./sdk/python
```

```python
import os
from geoledger import Client

with Client("http://127.0.0.1:7882", os.environ["GL_TOKEN"]) as client:
    project = client.create_project("城市道路")
    dataset = client.create_dataset(project.id, "roads")
    draft = client.create_workspace(project.id)
    draft.save(dataset.id, {
        "type": "Feature", "id": "road-1",
        "properties": {"name": "滨江大道"},
        "geometry": {"type": "Point", "coordinates": [120, 30]},
    })
    receipt = draft.publish("新增道路")
    rows = client.features(project.id, dataset.id, revision=receipt["revision"])
```

业务资源使用不可变数据类；要素、差异和审计使用普通字典。完整流程见
[Python 示例](../sdk/python/tests/smoke.py)。

## Rust

客户端 crate 为 [geoledger-client](../sdk/rust/Cargo.toml)。

```rust,ignore
use geoledger_client::Client;
use serde_json::json;

let client = Client::connect(endpoint, &token).await?;
let project = client.create_project("城市道路").await?;
let dataset = client.create_dataset(&project.id, "roads").await?;
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

模块为 [github.com/mapseekai/geoledger/sdk/go](../sdk/go/go.mod)。所有网络调用接受标准 `context.Context`。

```go
client, err := geoledger.Dial(endpoint, token)
if err != nil { return err }
defer client.Close()
project, err := client.CreateProject(ctx, "城市道路")
if err != nil { return err }
dataset, err := client.CreateDataset(ctx, project.ID, "roads")
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
const client = new Client(endpoint, token);
try {
  const project = await client.createProject("城市道路");
  const dataset = await client.createDataset(project.id, "roads");
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

## 常用操作

| 对象 | 业务方法（Python / Rust 命名；Go / TS 使用各自命名惯例） |
|---|---|
| Client | info、create_project、project、projects、set_member |
| Client | create_dataset、datasets、create_workspace、workspace、workspaces |
| Client | features、history、commit、audit、restore |
| Workspace | save、save_batch、delete、features、diff、conflicts |
| Workspace | publish、resolve、rebase、discard、refresh |

`save` 直接接收带字符串 `id` 的完整 GeoJSON Feature。批量编辑使用 SDK 自身的 `Edit`，
删除通过 `delete()` 或显式 `null` / `None` 表达，保存操作提供完整要素字段。
正式要素分页固定首次返回的 revision，后续查询带该版本及 next_after；草稿分页需核对 workspace_version。
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

## 浏览器 HTTP

`POST /api/v1/{operation}`，使用 snake_case 操作名，例如 `create_project`、`save`、`publish`、`features`。JSON 请求体字段对应 RPC 请求；HTTP 的 `feature` 为 GeoJSON 对象。列表响应为 JSON 数组，Features 返回 FeatureCollection 或单个 Feature。

```sh
curl --fail-with-body http://127.0.0.1:7881/api/v1/create_project \
  -H "Authorization: Bearer $GL_TOKEN" -H 'Content-Type: application/json' \
  --data '{"name":"roads"}'
```

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
| 资源不存在或无权查看 | 404 |
| 重复名称、版本、合并或幂等冲突 | 409 |
| 大小超限 / 执行容量耗尽 | 413 / 429 |
| 期限耗尽 | 504 |
| 存储不可用 | 503 |

Audit 仅项目 owner 可读。每页 1–1000 条、Feature 最大 16 KiB、最多 256 个属性；EPSG:4326，XY/XYZ。
标识采用有效文本字符，属性键和值使用 U+0000 以外的 JSON 文本；服务统一校验输入并返回业务错误。容量配置见 [生产运行](production.md)。

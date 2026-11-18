# 升级到 0.4.0-alpha.1

[项目概览](../README.md) · [API 契约](api.md) · [存储扩展](storage.md) · [发布流程](release.md)

本版本清理旧调用方式、宽松输入和旧发行专属分支。服务端、CLI、四语言 SDK 与 Web 控制台应作为一个版本组合升级。本文描述源码候选的迁移要求；实际包和镜像是否已发布，以对应发布记录为准。

## 升级影响

| 使用方 | 必须调整 | 保持的能力 |
|---|---|---|
| Rust 构建 | 最低 Rust 从 1.88 提高到 1.89；重新解析并使用提交的锁文件 | 常规 CI 仍使用固定工具链，MSRV 有独立检查 |
| Rust SDK | 连接使用三参数 `Client::connect`，消息限制放入 `ConnectOptions` | 私有 CA、明文显式授权、30 秒默认请求期限、连接克隆 |
| Rust 嵌入服务 | `Application::initialize`；构造 `Service` 时配置限制 | 当前库格式检查与初始化、共享限额和 draining 状态 |
| 自定义存储 | 实现显式的 schema 与存在性探测方法，使用具名编辑上下文 | 共用 Application、事务、权限、版本、历史和回执合同 |
| HTTP/动态 JSON 调用者 | 只发送当前操作字段，显式提供必填值；`info` 发送 `{}` | 操作规定的可选字段、分页默认值、自由 GeoJSON 属性 |
| 四语言 SDK/Web 错误消费者 | 使用当前 envelope 和 `deadline_exceeded`；更新明确发布拒绝的状态判断 | 未知发布恢复、身份恢复、冲突处理与原请求幂等重放 |
| 发布维护者 | 使用普通 tag、source-release 或 release-resume | 构建验证、签名、凭证门禁、历史镜像审计 |

## Rust SDK 的统一连接入口

以下入口被删除：两参数 `Client::connect`、`connect_with_timeout`、`connect_with` 和 `connect_with_message_limits`。不提供转发别名。

```rust,ignore
use geoledger_client::{Client, ConnectOptions, MessageLimits};
use std::time::Duration;

let client = Client::connect(endpoint, &api_key, ConnectOptions::default()).await?;

let options = ConnectOptions {
    timeout: Duration::from_secs(45),
    ca_pem: Some(private_ca_pem),
    message_limits: MessageLimits::new(32 * 1024 * 1024, 128 * 1024 * 1024)?,
    ..ConnectOptions::default()
};
let client = Client::connect(endpoint, &api_key, options).await?;
```

`MessageLimits::default()` 的编码与解码上限均为 **67,108,864 字节（64 MiB）**，替代旧的 `usize::MAX`。计量对象是序列化后的 Protobuf 消息，不含 gRPC frame header；客户端不启用压缩。它不是总进程内存限制，也不会提高服务端请求/响应预算。两个方向都要求正数，可用 `MessageLimits::new` 显式设置更大的值。

CLI 使用 `--max-encoding-bytes`、`--max-decoding-bytes`，或环境变量 `GL_CLIENT_MAX_ENCODING_BYTES`、`GL_CLIENT_MAX_DECODING_BYTES` 设置相同限制；默认各 64 MiB，零值会被拒绝。

需要读取大页的应用应按实际消息大小设置解码上限，或减小查询分页。发布响应可能在事务提交后才因客户端限制无法解码；此时必须保留原发布对象，使用更大解码限制的新客户端重试同一 request ID、版本和 payload。不要把它当成一次确定失败而创建新发布。

`ConnectOptions` 完整 struct literal 需要新增 `message_limits`；优先显式填写业务关心的选项，其余使用 `..Default::default()`。SDK `Error` 使用 `Error::new` 构造，新增的私有诊断来源意味着不能在外部通过 struct literal 构造；公开错误消息保持脱敏。

## JSON 以业务请求为边界

Rust SDK `execute`、CLI 的动态 JSON 入口、HTTP 业务请求和认证操作都拒绝未知字段、缺少必需字段以及错误 JSON 类型。生成的 Protobuf DTO 不再被当作动态 JSON 输入校验器，因此不会依赖 proto3 的默认空字符串、零或 false 来补出调用意图。

例如，归档与恢复必须明确给出布尔值：

```json
{"project":"project-id","archived":false}
```

`{"project":"project-id"}`、`{"project":"project-id","archived":"false"}` 和多余字段都返回参数错误。`info` 只接收 `{}`。查询参数的实际默认值仍按各操作定义生效，不要求为每个可选字段填写零值。严格字段检查只针对请求结构；Feature 的 `properties` 仍可以包含合法的业务键和精确 JSON 数值。

字段保存的含义不变：省略 `schema_edits` 保留原意图，显式 `[]` 清空原意图。认证操作也区分 presence：更新 API Key 时省略 `expires_at` 保留到期时间，显式 `null` 清除。到期时间使用 RFC 3339，带偏移输入先转换到 UTC，然后以三位毫秒和 `Z` 保存；必须在未来，且 UTC 年份为 1–9999。本版本明确拒绝闰秒 `:60`，不会静默把它改成前一纳秒。已有规范化的凭证时间和合法 Argon2id PHC 密码记录可继续使用。

## 发布拒绝与未知结果

源表 CHECK、NOT NULL、外键和数值范围拒绝发生在指定源写入/转换/约束检查边界且在发送 COMMIT 前 时，返回：

```text
HTTP 400 / gRPC InvalidArgument
code = publication_attempt_rejected
uncertain = false
```

低层 SDK 与工作区层都直接表达本次确定失败，删除原来先按 503/Unavailable 报不确定、再由高层改写的兼容路径。这里的确定性只属于**本次尝试**。

| 先前状态 | 本次结果 | 调用者处理 |
|---|---|---|
| 没有待确认发布 | 明确源表拒绝 | 释放新建 intent；修正原草稿，再创建新发布 |
| 已有未知结果 | 源表拒绝、输入拒绝、身份/权限/可见性失败或普通/合并冲突 | 保留原 request ID、版本、说明和 payload；恢复条件后原样重试 |
| 任意 | 断链、超时、取消、响应丢失或消息解码限制 | 保留未知结果，不能据此断言数据库回滚 |
| 已有原请求 | 成功回执或回执重放 | 使用该回执；不重复创建提交 |
| 已有未知结果 | 可信业务详情中的 `workspace_version_conflict`，且 uncertain=false | 原预期工作区版本已永久失效，允许释放；随后读取当前状态 |

不要仅根据最后一次异常的 `uncertain=false` 清理一个先前未知的发布。自建 HTTP 客户端和自行保存 `Publication` 的低层 SDK 调用者需要自己保留这段生命周期；工作区对象和 Web 控制台已负责它。服务端继续在项目 HEAD 锁前后查回执，保存/历史/审计/回执仍在一个事务中提交。只有真正的工作区预期版本失效或已关闭检查产生 `workspace_version_conflict`；`expected_head` 变化和合并冲突仍是普通 conflict。

## 恢复未知发布

普通或合并冲突只说明当前尝试失败，不能终结先前未知结果。例如旧请求排队时，别的工作区可能先改变项目 HEAD，导致重试发现合并冲突；之后又恢复相关字段值，排队的旧请求仍可能发布成功。

持续需要修正时，使用以下顺序保留可确认性：

1. 保留原 `Publication` 的 request ID、版本、说明和完整 payload；不要改原请求，也不要先建立另一笔发布。
2. 用独立的当前工作区句柄读取同一工作区并检查现有状态/冲突。通过有版本检查的 `save`、`resolve` 或 `rebase` 提交明确的草稿修正；成功会原子增加工作区版本。这一步是草稿操作，不是发布。
3. 用原 intent 重试原发布。如果旧请求已经提交，回执优先返回原成功；如果版本推进先完成，返回确定的 `workspace_version_conflict`，旧预期版本不能再提交。若仍收到网络、权限或其他错误，继续保留原 intent。
4. 确认原成功，或收到上述版本失效证明后，再读取最新工作区状态。需要发布修正后的草稿时，显式创建新的发布意图。

同一 Workspace 对象在 pending 时仍禁止编辑，避免它悄悄覆盖原 intent。此恢复步骤使用单独的当前句柄及服务器原子版本检查；不依赖超时后等待一个固定时长，也不假设取消客户端 future 等于回滚服务器事务。

## 错误消费者

HTTP 业务错误唯一结构为 `{"error":{"code":"…","message":"…","request_id":"…"}}`，可带 `conflicts`。BFF 不再解释顶层 `code/message` 或字符串 `error` 旧形状；`forbidden`、`bad_request` 等旧别名不再当作当前业务码映射。标准码为 `permission_denied`、`invalid_argument` 等。

四语言 SDK、Web 与引擎统一使用 `deadline_exceeded` 表达期限耗尽，替代部分客户端以前返回的 `timeout`。没有业务详情的 gRPC `OutOfRange` 统一为 `resource_exhausted` 且 `uncertain=true`；有效结构化业务详情仍优先保留业务码。无有效当前详情的 `Aborted`、`AlreadyExists`、`FailedPrecondition` 和 HTTP 409 也保持不确定；gRPC envelope 的 code 必须与传输状态一致，且业务 code 非空。代理的空响应、HTML 或普通文本错误继续按 HTTP 状态保守处理，错误内容不作为当前业务详情或直接回显给用户。

## Rust 服务嵌入与自定义后端

```rust,ignore
let app = geoledger_engine::Application::new(storage);
app.initialize()?;
let service = geoledger_server::Service::new_with_limits(app, limits);
```

原 `Application::migrate()` 改为 `initialize()`，不保留旧名。原实现只创建当前格式的新库或检查已有格式，并没有旧格式升级器；这次命名反映实际行为。`StorageBackend::initialize` 的合同不变。

`Service::new(app)` 仍提供默认限制，`new_with_limits(app, limits)` 提供一次性自定义构造。原 `Service::new(app).with_limits(limits)` 改为上面的形式。已经运行的克隆共享并发槽、内存预算、速率限制、指标与 draining 状态，不能通过单独修改克隆的配置产生不同限额。

自定义 backend 需要逐项迁移：

- `WorkspaceRepository::edit_base` 返回 `EditContext { resolved, base, draft }`；`DraftState` 区分没有草稿、明确删除和草稿值，不再依赖六位置 `Row`。
- Dataset/Publication 的字段定义、字段意图和发布 schema 方法必须显式实现。没有 schema 能力的后端应明确报告缺少该能力并拒绝非空 schema 请求，同时继续支持合法空清理与普通要素发布。
- `FeatureRepository::feature_exists` 必须实现。它在同一事务快照、可见性、游标和过滤条件下探测匹配键，不应为判断下一页而物化不会返回的 properties/geometry。旧的 `feature_page(limit=1)` 默认替代被移除。
- SQL 的参数、连接和事务实现细节收回内部；通过 `SqlStorage::new(Storage, StorageOptions)` 构造适配器，业务层使用语义仓储操作。维护、导入导出、TLS 和指标接口按当前存储合同使用。

参考 [schema-less 测试后端](../crates/engine/tests/support/schema_less_backend.rs) 和 [外部 API 编译测试](../crates/engine/tests/public_api.rs)。测试 fixture 说明扩展合同，并不代表完整生产后端。

## 存储、发布与依赖

存储格式继续为 **19**；没有新表结构或旧库转换。本次无需因为 API 清理而重建现有格式 19 数据库。旧格式仍被明确拒绝，当前格式备份恢复、PostGIS 原表绑定重建、动态字段能力、历史回放和精确数字均保留。gRPC 包仍为 `geoledger.v1`、HTTP 仍为 `/api/v1`；字段编号和生成绑定没有因版本名而重排。

alpha.4 的固定源码恢复工作流、alpha.7 的一次性 SDK 补发 jobs，以及它们专属的脚本、测试和执行例外被删除。新的发布使用 [发布流程](release.md) 中的通用路径；旧版本信息保留在 Git 历史、变更记录和带日期的验证报告。保留历史记录不意味着当前程序仍执行旧版本兼容路径。

依赖只做有明确依据的定向维护：rusqlite 0.41.1 / libsqlite3-sys 0.39.1、pulldown-cmark 0.13.6、rustix 1.1.6，chrono 替换为 time 0.3.55，Next.js 升为 16.4.6，Go 模块最低补丁为 1.27.4；TypeScript 原生编译器的 Linux amd64/arm64 重建锁也固定为该补丁，并更新、核验官方归档大小与摘要。实际解析版本以锁文件为准。Rust 1.89 是新构建要求；四生态依赖审计、原生 SQLite/PostGIS、SDK 和 Web 的验证按 [开发文档](development.md) 执行。更新源码和锁文件不代表线上实例已经部署更新。

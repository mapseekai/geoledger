# SDK 封装与独立代码审查

[SDK 用法](api.md) · [重构验证](production-review.md)

## 接口调整

四语言 SDK 的公开入口仅包含业务客户端、业务模型和业务错误。
生成绑定置于私有目录：Go 使用编译器约束的 `internal`，Python 使用 `_internal`，
TS 包通过 `exports` 限定入口，Rust 通过业务客户端封装协议模块与 transport。
TS 方法直接返回 Promise；全部语言接受普通 GeoJSON 数据。

工作区对象维护乐观版本，保存成功后更新版本，失败后保留原状态。
Rust 使用可变借用，Go 使用互斥锁，Python 使用 RLock，TS 对同一句柄的修改进行并发校验。
多个独立句柄的竞态仍由服务端版本检查处理。

发布时生成不可变的业务发布意图，超时等未知结果保留原请求；同一说明重试复用相同 ID、
项目、工作区和版本。未知结果期间保持原修改上下文与说明。可序列化的发布意图支持跨进程恢复，
调用方明确发起写入重试，通过版本检查协调并发编辑。

## 审查范围与发现

由独立子 agent 检查 engine 授权、事务适配器、发布与合并、恢复和分页；服务端认证、
HTTP/gRPC 与控制台；四语言 SDK 的公开接口、JSON 精度、删除语义、发布重试和句柄并发。
主 agent 复核发现并完成封装、实际服务联调及发布准备。

| 优先级 | 能力与解决的问题 | 实现及验证证据 |
|---|---|---|
| P1 | 大量复杂要素冲突可分页查询，客户端可逐页完成冲突解决。 | 错误详情达到 4 KiB 时返回 head/version/total 等摘要，完整要素走普通分页查询。真实 HTTP/2 测试使用 8 KiB metadata 限制，取得业务错误并分页读完冲突。 |
| P2 | 数据集名称冲突返回统一业务码，应用可引导用户调整名称。 | SQLite UNIQUE/PRIMARY KEY 与 Postgres 23505 映射为 409/conflict。双后端验证事务完整回滚、审计保持原状及后续写入；公开消息采用业务描述。 |
| P2 | 属性文本在 SQLite 与 PostGIS 中获得一致的输入校验结果。 | 共同应用层递归校验实际 NUL，统一返回 400；字面字符序列 `\\u0000` 可保存发布。双后端验证草稿版本在校验失败后保持原状。 |
| P2 | 用户属性按原始 JSON 保留，包括 `geojson` / `detail_json` 等普通字段。 | 转换协议声明的要素/变更/审计字段，解包后的用户 JSON 保持原样。四语言回归包含这些普通属性和精确 u64。 |
| P2 | 保存和删除采用明确的编辑意图，保护要素数据。 | SDK 要求显式 feature，Rust 使用必填反序列化器、Go 在序列化前检查 RawMessage，Python/TS 检查字段存在。显式 delete/null 表达删除。 |
| P2 | 大整数及整值小数/指数形式可精确解析和往返保存。 | TS 精确解析整值并使用 bigint，对非零下溢和数值表示范围进行校验；输入对象中的大整数使用 bigint 或原始 JSON 文本。 |

## 验证入口

- [Rust SDK 测试](../sdk/rust/tests/client.rs)：真实连接验证未知发布结果复用原请求；库内测试检查任意属性和缺字段。
- [Python 边界测试](../sdk/python/tests/test_client.py)：6 项，涵盖不透明属性、大整数、显式删除、版本、重试和发布失败。
- [TS 边界测试](../sdk/ts/test/client.test.cjs)：7 项，增加公开入口隔离、数值形式和同句柄并发。
- [Go 边界测试](../sdk/go/client_test.go)：4 项，并通过 race 检查。
- [大冲突回归](../crates/server/tests/rpc_errors.rs)：真实 HTTP/2 错误元数据与冲突分页。
- [存储一致性](../crates/engine/tests/conformance.rs)：相同测试分别运行 SQLite / PostGIS。
- [四语言联调](../scripts/test-sdks.sh)：两种实际服务上的创建、保存、版本推进、显式删除、发布与原请求重试、大整数、业务错误；Python 另覆盖完整冲突解决与恢复。
- [全仓检查](../scripts/check.sh) 与 [浏览器回归](../scripts/test-console.py)：覆盖其余服务与控制台行为。

生成绑定已验证可重复生成，Python wheel 构建后进行了实际安装和服务联调；TS 完成构建与包内容检查。
测试使用隔离服务及测试数据，验证范围为 SDK 正确性与服务联调。百万要素容量基线见重构验证记录；
生产 SLA、Windows 与目标环境部署通过专项验收确认，容器控制台验证见 [Web 验证记录](web-review.md)。

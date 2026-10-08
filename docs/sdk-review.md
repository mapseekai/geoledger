# SDK 封装与独立代码审查

[SDK 用法](api.md) · [重构验证](production-review.md)

## 接口调整

四语言 SDK 的公开入口仅包含业务客户端、业务模型和业务错误。
生成绑定置于私有目录：Go 使用编译器约束的 `internal`，Python 使用 `_internal`，
TS 包通过 `exports` 限定入口，Rust 不再重导出协议模块或公开 transport 字段。
TS 方法直接返回 Promise；全部语言接受普通 GeoJSON 数据。

工作区对象维护乐观版本，保存成功后更新版本，失败后保留原状态。
Rust 使用可变借用，Go 使用互斥锁，Python 使用 RLock，TS 拒绝同一句柄同时进行多个修改。
多个独立句柄的竞态仍由服务端版本检查处理。

发布时生成不可变的业务发布意图，超时等未知结果保留原请求；同一说明重试复用相同 ID、
项目、工作区和版本。未知结果期间拒绝修改或更换说明。提供可序列化的发布意图以支持跨进程恢复，
SDK 不自动重试写入或静默覆盖并发版本。

## 审查范围与发现

由独立子 agent 检查 engine 授权、事务适配器、发布与合并、恢复和分页；服务端认证、
HTTP/gRPC 与控制台；四语言 SDK 的公开接口、JSON 精度、删除语义、发布重试和句柄并发。
主 agent 复核发现并完成封装、实际服务联调及发布准备。

| 优先级 | 触发条件与影响 | 修复及证据 |
|---|---|---|
| P1 | 多个大要素发生合并冲突，完整冲突通过错误 metadata 传输。20 个约 7 KB 要素产生约 309 KB metadata，客户端只收到传输大小错误，丢失业务冲突。 | 错误详情达到 4 KiB 时仅保留 head/version/total 等摘要，完整要素走普通分页查询。真实 HTTP/2 测试使用 8 KiB metadata 限制，仍能取得业务错误并分页读完冲突。 |
| P2 | 重复数据集名称触发数据库唯一约束，被归类为 503，调用方误判为服务故障。 | SQLite UNIQUE/PRIMARY KEY 与 Postgres 23505 映射为 409/conflict。双后端测试检查无残留数据、无额外审计、事务后仍可继续写入，公开消息不泄露数据库诊断。 |
| P2 | 属性值或嵌套键含实际 U+0000，SQLite 可存而 PostGIS jsonb 报 503。 | 在共同应用层递归拒绝实际 NUL，统一返回 400；字面字符序列 `\\u0000` 仍可保存发布。双后端验证失败不推进草稿版本。 |
| P2 | SDK 回复转换递归识别用户属性中的 `geojson` / `detail_json`，或对同一数据转换两次，导致合法属性被误解析。 | 只转换协议声明的要素/变更/审计字段，解包后的用户 JSON 保持原样。四语言回归包含这些普通属性和精确 u64。 |
| P2 | 省略编辑的 feature 被生成绑定默认解释成删除。Rust Option 反序列化也可能把缺字段变成 None。 | SDK 要求显式 feature，Rust 自定义必填反序列化器、Go 在序列化前检查非空 RawMessage，Python/TS 检查字段存在。显式 delete/null 仍可删除。 |
| P2 | JS 大整数、整值小数/指数形式或非零下溢被普通 Number 静默舍入。 | TS 精确解析整值并使用 bigint，拒绝非零下溢和无法安全表示的相关数值；输入对象中的不安全整数要求 bigint 或原始 JSON 文本。 |

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
测试使用隔离服务及测试数据，不将本轮正确性回归解释为生产 SLA。此前的百万要素容量数据保留在重构验证记录，
本次 SDK 封装没有重新执行百万要素测试。Windows、容器部署及真实目标环境验收仍按该记录的交付边界执行。

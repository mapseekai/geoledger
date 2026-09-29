# GeoLedger 验证记录

## 当前格式回归：2026-09-29

本轮验证 GeoLedger 格式 3、统一命名与默认作者 mapseekai。环境为 macOS Apple Silicon、Rust 1.92.0，以及独立临时容器中的 PostGIS 3.6.1。测试数据库为 `geoledger_test`，数据库角色为 `mapseekai`，测试完成后停止并移除该临时容器。

`cargo test --workspace --offline -- --include-ignored --test-threads=1` 实际执行 **75 项测试，75 通过、0 失败、0 跳过**，包含真实 PostGIS 和网络传输测试。

覆盖当前仓库目录、对象域、schema 版本和稳定身份、SQLite 标识、状态与 pending journal、首次导入后的字段历史恢复、文件锁、重复初始化保护、对象校验，以及 CLI / JSON / HTTP / gRPC / Thrift 的作者默认值。CLI 同时验证显式参数高于环境变量、环境变量高于默认值；服务入口验证显式作者按请求保留。

PostGIS 回归覆盖导入、精确属性和几何、记录跟踪、字段新增/删除/改名/类型变更、外部 DDL、三方合并、冲突处理、唯一值交换、结构和记录恢复、journal 协调、数据库超时及校验失败后的事务状态。

## 可重复执行的检查

在项目根目录运行 `./scripts/check.sh`。基础检查包含文档链接、格式、Clippy 和工作区测试；数据库测试通过 `GL_TEST_DATABASE_URL` 显式启用，连接的数据库名为 `geoledger_test`，测试使用随机 schema。配置变量采用 [.env.example](../.env.example) 模板并填写实际连接。

主要回归源码为 [CLI 测试](../crates/cli/tests/cli.rs)、[应用离线测试](../crates/app/tests/offline.rs)、[PostGIS 测试](../crates/app/tests/postgis.rs)、[存储测试](../crates/storage/tests/storage.rs)、[HTTP/gRPC 测试](../crates/server/tests/transports.rs)与[Thrift 测试](../crates/server/tests/thrift.rs)。

## 历史开发基线

2026-09-21 的基础验证执行 26 项测试，并完成 CLI、HTTP、gRPC 和 Rust 库的开发端到端调用。2026-09-22 的字段及批处理回归执行 40 项测试，审查修复回归执行 50 项，Thrift 接入阶段执行 53 项；各数字对应当时的代码阶段。

历史大数据测量覆盖 9,384 个多面要素和 483,268 个三维多线要素，记录初始化、增量提交、切换、快进合并、revert、字段操作和 fsck，并使用全表摘要核对恢复结果。详细规模、耗时和资源口径见 [大数据验证](big-data-verification.md)，算法与内存改进见 [审查改进](rust-review-fixes.md)。

历史计时作为对应样本与环境下的开发基线；本轮格式回归验证功能和一致性。更大规模、并发、跨平台及故障注入的工作安排见 [开发计划](roadmap.md)。

## 本轮检查汇总

| 检查 | 结果 |
|---|---|
| 文档链接及命名检查 | 11 份文档，66 处仓库内引用和 6 个公开 HTTP 链接通过 |
| cargo fmt --all -- --check | 通过 |
| cargo clippy --workspace --all-targets --offline -- -D warnings | 通过 |
| cargo test --workspace --offline | 47 通过，28 项数据库测试按默认策略跳过 |
| 显式启用全部数据库测试 | 75 通过，0 失败，0 跳过 |
| cargo build --workspace --bins --examples --offline --locked | 通过 |
| CLI 版本 | gl 0.1.0 |

文档检查由 [check-docs.py](../scripts/check-docs.py) 执行，基础检查入口为 [check.sh](../scripts/check.sh)。服务地址使用 README/API 中显式启动的本地监听，公开源码链接指向固定提交。

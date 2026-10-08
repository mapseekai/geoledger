# 服务化重构与验证记录

[项目概览](../README.md) · [生产运行](production.md) · [开发验证](development.md)

## 本轮结构

按统一基础服务重新组织：唯一 `geoledger-server`，默认 SQLite、可选 PostGIS；`gl` 为远程 gRPC 客户端；Go/Rust/Node.js/Python 提供业务 SDK，内部绑定共享统一 Protobuf；浏览器管理使用 HTTP。

移除本地仓库 CLI、旧 SQLite 对象仓库、旧业务表跟踪入口、Thrift 与旧通用 JSON RPC 服务。保留并独立提取属性/几何三方合并算法，业务状态与历史由新存储接口管理。格式 4 使用新库，源码中没有旧格式读取或自动升级逻辑。

存储接口包含原子事务、项目/工作区锁定语义、历史可见性、发布暂存、幂等收据和审计；新后端通过实现接口注入共同 Application。协议和存储扩展方式见 [API](api.md)、[存储接口](storage.md)。

## 已验证的范围

完整 `scripts/check.sh` 已通过：43 个 Rust 测试执行结果、6 个浏览器脚本测试，以及 fmt、Clippy 和文档链接检查。此前两个百万要素测试另行显式执行通过；本次 SDK 边界测试额外通过 Python 6 项、TS 7 项、Go 4 项及 Go race 检查。

- SQLite 与 PostGIS 执行相同业务一致性场景：权限、幂等、批次回滚、工作区竞态、并发发布、固定历史快照、几何 bbox 遮蔽、冲突与重基、撤销、过期解决选择、审计分页、千条大要素发布和分页冲突。
- 默认 SQLite 真实服务进程：HTTP 与 gRPC 共存、未认证拒绝、伪造作者拒绝、20 路并发保存/发布、相同请求重试、停止后重启读取已提交版本。
- SQLite 故障注入：写锁期限、提交时延迟外键失败、全部历史和收据回滚、原请求修复后重试、不可变记录约束。
- 四语言 SDK 分别连接 SQLite 与非超级用户账号的 PostGIS 服务，完成发布、幂等重试、读回及 u64 最大值精度检查；Python 同时验证业务错误、冲突解决和恢复。
- 浏览器脚本回归检查原始数字、原发布请求重试、凭证内存保存、结构化错误显示和会话结束后迟到响应的隔离。
- Chromium 真实控制台回归覆盖独立登录页、无效 / 过期令牌、创建项目与数据集、草稿编辑、精确大整数、丢失发布响应后的原请求重试、历史 / 审计 / 权限、游标分页、退出清理及手机布局。

验证环境为 Linux x86_64、Rust 1.92、Node 22、Python 3.12、PostgreSQL 17.11 / PostGIS 3.5。仓库中 CI 同时包含 Rust 1.88 与 Windows 构建任务；Rust 1.88 的全工作区/all-targets 检查已在云端通过；Windows 任务留给 CI 执行。真实 Chromium 管理界面也已完成创建项目、数据集、工作区、保存、发布和浏览流程，没有 JavaScript 错误。

本轮 SDK 封装与独立审查的发现、修复和专门测试见 [SDK 审查记录](sdk-review.md)。

## 容量方法

以下是此前服务化重构的容量基线，本次 SDK 封装没有重新运行百万要素测试。

[capacity.rs](../crates/engine/tests/capacity.rs) 是显式启用的百万要素 / 20 个独立身份测试。SQL 播种一百万个 Point，每条包含 512 字节属性，然后 20 个用户各做五轮：深分页查询、创建草稿、保存、发布、重复原发布请求、读回，最终核对 HEAD 与历史不变性。

```sh
cargo test --locked -p geoledger-engine --test capacity -- --ignored --nocapture
GL_CONFORMANCE_BACKEND=postgis cargo test --locked -p geoledger-engine --test capacity -- --ignored --nocapture
```

PostGIS 用例要求隔离的 `geoledger_test` 数据库。SQLite 用例用临时目录并自动清理。测试至少需要数 GB 可用空间；直接 SQL 播种不代表百万要素公共接口导入性能。

本机 debug 构建、百万 Point、约 623 MB GeoJSON（按每条最大位数估算）、20 个独立用户、100 轮完整操作，结果如下。播种时间不计入操作耗时。

| 后端 | 播种 | 100 轮总时间 | 单轮 P50 | 单轮 P95 | 单轮最大 |
|---|---:|---:|---:|---:|---:|
| SQLite | 72.365 s | 3.055 s | 0.213 s | 1.830 s | 2.713 s |
| PostGIS | 60.039 s | 4.192 s | 0.685 s | 1.609 s | 2.052 s |

本轮首先发现 SQLite 发布中的相关删除子查询只按 project 扫描候选，百万要素下导致写锁排队超时。修复为 `(dataset, feature_id)` 变更键集合查询后，两种后端均通过上述测试。历史与当前要素的索引定位是此次验证重点。

这些数字来自云端共享机器，不是后端优劣排序或 SLA；没有覆盖复杂大面、深历史、长时间写入、全范围 bbox 扫描或公共接口百万要素初次导入。真实 gRPC 的 20 路并发另由服务进程测试覆盖。旧格式的性能数字不作为本版验收依据。

## 交付边界

本轮交付源码、业务 SDK 及内部生成绑定、构建脚本、部署示例和测试。Linux release 服务端/CLI 已构建，并用实际安装的 Python SDK 连通 release 服务验证发布、重试及读回。公开 SDK 包、容器注册表镜像与正式发布需要独立发布流程。Docker 镜像构建已尝试：拉取基础镜像成功，但容器构建阶段无法解析 deb.debian.org，默认网络与 host 网络均失败，因而镜像运行验证留待具备容器 DNS 的环境。systemd 定义应在目标部署环境实际演练。

生产上线还需结合实际几何类型、磁盘、数据增长、代理 TLS、企业身份、备份恢复和目标延迟验收。默认 SQLite 的写入串行与 PostGIS 的并行事务属于不同容量模型；客户端业务语义保持一致。

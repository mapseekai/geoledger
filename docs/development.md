# 开发说明

[项目概览](../README.md) · [快速开始](getting-started.md) · [功能指南](user-guide.md) · [API](api.md)

## 架构

| 层 | 职责 |
|---|---|
| `crates/core` | 版本对象、持久化树、提交图、差异与合并算法 |
| `crates/storage` | SQLite WAL 对象库、Zstd 压缩、文件锁及恢复日志 |
| `crates/postgis` | 表检查、记录编码、字段演进、跟踪触发器及数据库事务 |
| `crates/app` | 通过 Application / Command 编排全部版本操作 |
| `crates/server`、`crates/thrift-gen` | HTTP / gRPC / Thrift 服务及协议绑定 |
| `crates/cli` | 命令解析与服务启动 |

核心算法通过 `ObjectStore`、`WorkingCopyProvider`、`WorkingCopyTransaction` 对接存储和工作副本。异步入口使用阻塞工作线程调用应用层。

PostGIS 保存可编辑工作副本；SQLite 保存历史对象和分支。提交采用“对象与 pending 持久化 → 数据库提交 → 分支状态发布”的顺序，`recover` 通过操作标记协调恢复。数据和字段恢复持续保留跟踪触发器及表锁。

当前持久化格式为 3，本地目录为 `.geoledger`，数据库元数据为 `_geoledger`。对象读取校验类型和哈希，稳定字段 ID 用于结构历史与合并。开发验证使用新建仓库和专用测试表。

## 构建

在项目根目录准备 Rust、C 编译器和 protoc；Linux 构建还使用 OpenSSL 开发包与 pkg-config。工具链与依赖要求见 [Cargo.toml](../Cargo.toml) 和 [Cargo.lock](../Cargo.lock)。

macOS / Linux 构建全部入口：

```bash
cargo build --workspace --bins --examples --locked
cargo install --path crates/cli --locked
```

Windows 在 MSVC 环境构建 CLI、HTTP 和 gRPC：

```powershell
cargo build --release --locked --target x86_64-pc-windows-msvc -p geoledger-cli --no-default-features
```

## 验证

macOS / Linux 运行 `./scripts/check.sh`，执行文档、格式、Clippy 和工作区测试。设置 `GL_TEST_DATABASE_URL` 指向专用 `geoledger_test` 数据库后，脚本同时执行 PostGIS 集成测试；测试数据使用隔离环境。

Windows 使用对应功能组合：

```powershell
python scripts/check-docs.py
cargo fmt --all -- --check
cargo test --locked --workspace --exclude geoledger-thrift-gen --no-default-features
```

## Windows 打包

[Windows 工作流](../.github/workflows/windows-cli.yml) 完成原生构建、测试、PowerShell 自检和运行时依赖检查，再由 [打包脚本](../scripts/package-windows.py) 生成 ZIP 及 SHA256。安装包包含可执行文件、统一指南、配置模板与测试材料。

[文档检查](../scripts/check-docs.py) 校验本地链接及章节引用。文档按“快速开始、功能指南、API、开发说明”维护，每项内容集中在对应页面；操作示例使用实际配置或由准备步骤创建的资源。

## 中心版

独立包 [geoledger-center](../crates/center/Cargo.toml) 为 `0.2.0-alpha.1`，入口 `gl-center`，
[CenterApplication](../crates/center/src/lib.rs) 处理全部中心操作，HTTP 层只负责认证、限流、大小限制和阻塞任务调度。
中心版复用 core 的 `merge_record`；中心属性使用带 JSON Pointer 转义的 `/properties/` 字段，几何使用 `/geometry`，
JSON 值编码为文本单元，几何编码为保留坐标顺序与 Z 的 XDR EWKB。这是中心格式 1 的内部映射，本地 core 编解码保持原样。

[事务迁移](../crates/center/src/schema.sql) 建立项目范围复合外键、成员、工作区、增量、当前要素、时态历史、提交、幂等记录和审计表。
历史值与提交由数据库触发器保护；历史区间在后续提交中仅关闭一次。发布在同一连接的单一事务中锁项目行及工作区行，
重新合并后一起更新要素、历史、提交、HEAD、审计、幂等结果和工作区状态。草稿仅锁自身工作区及成员身份。
项目采用 `FOR NO KEY UPDATE`，与草稿写入的外键 KEY SHARE 锁兼容；成员管理与发布采用一致的锁顺序；bootstrap 的事务 advisory lock 仅用于协调迁移。
服务专用写角色是运行前提，数据库所有者仍拥有管理权限。

离线验证：

```sh
python3 scripts/check-docs.py
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --offline -- -D warnings
cargo test --workspace --offline
# 由测试操作者配置专用 GL_TEST_DATABASE_URL，数据库名必须为 geoledger_test。
cargo test -p geoledger-center --test postgis --offline -- --ignored --test-threads=1
```

[中心集成测试](../crates/center/tests/postgis.rs) 在写入前核验数据库名，使用随机项目和身份，可共享中心 schema。
`migrate` 需要预装 PostGIS；测试保留随机项目的不可变历史，整个测试数据库可由测试操作者重建。
`scripts/check.sh` 在配置测试 URL 时包含中心测试。中心 crate 直接使用现有 Axum、postgres/native-tls 和 core，
Windows 本地 CLI 构建链及 Volo 特性边界保持原样。浏览器测试台由中心服务内嵌提供，令牌保留在内存中，接口按同源部署。


中心版 Windows 发布由 [独立工作流](../.github/workflows/center-windows.yml) 构建 `gl-center.exe`，
运行原生测试并检查系统依赖，再由 [中心打包脚本](../scripts/package-center.py) 生成带指南、构建信息和 SHA256 的安装包。
两种可执行文件独立发布，共用核心字段合并算法。

中心二进制的 `--build-info` 提供编译时的提交、目标、配置和运行时信息；打包脚本核对干净工作区与该信息，
并记录可执行文件 SHA256，保证安装包说明对应实际构建。

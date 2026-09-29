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

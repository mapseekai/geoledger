# 快速开始

[项目概览](../README.md) · [控制台操作](console.md) · [API 与 SDK](api.md) · [生产运行](production.md)

本指南从新克隆的仓库启动默认 SQLite 服务，验证 CLI 连接，并选择 Web、SDK 或 PostGIS 接入。命令从仓库根目录执行；服务进程保持在独立终端运行。

## 获取代码

克隆步骤见 [项目概览](../README.md#获取代码)。进入 `geoledger` 后，按下列步骤准备工具。

## 环境准备

Rust 最低版本由 [Cargo.toml](../Cargo.toml) 定义为 1.88；主 CI 和 Dockerfile 使用 1.92。通过已安装的 rustup 配置一致的工具链：

```sh
rustup toolchain install 1.92.0 --profile minimal --component rustfmt,clippy
rustup override set 1.92.0
```

工具安装入口：[Rust / rustup](https://www.rust-lang.org/tools/install)、[protoc](https://github.com/protocolbuffers/protobuf/releases)、[Node.js 22](https://nodejs.org/en/download)、[Python 3.10+](https://www.python.org/downloads/)。Web 使用 Node.js 22，仓库脚本使用 Python 3 和 Bash。

### Linux

Ubuntu/Debian 安装 C 工具链、protoc 和连接库：

```sh
sudo apt-get update
sudo apt-get install -y build-essential protobuf-compiler pkg-config libssl-dev python3 python3-venv curl
```

### macOS

使用 Xcode 命令行工具和已安装的 Homebrew：

```sh
xcode-select --install
brew install protobuf pkg-config openssl@3 python@3.12 node@22
export PATH="$(brew --prefix node@22)/bin:$PATH"
export PKG_CONFIG_PATH="$(brew --prefix openssl@3)/lib/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
```

### Windows

安装 Visual Studio Build Tools 的 C++ 工具链、Rust MSVC 工具链、Python 和 Node.js 22。将 protoc 的 `bin` 目录加入 PATH。原生构建与测试命令使用 Cargo；仓库 `.sh` 脚本在 Bash 环境运行。主 [CI](../.github/workflows/ci.yml) 包含 Windows 构建和测试配置。

### 检查工具

```sh
rustc --version
cargo --version
protoc --version
python3 --version
```

使用控制台时再检查 `node --version` 和 `npm --version`。SDK 生成及 Go/Python 联调的额外工具见 [开发指南](development.md)。

## 构建并启动 SQLite 服务

```sh
cargo build --locked --bins
./target/debug/geoledger-server --data-dir ./geoledger-data
```

首次启动创建 `geoledger-data/geoledger.sqlite3`、只含令牌摘要的 `geoledger-data/tokens.json` 和明文管理员凭证 `geoledger-data/admin-credentials.json`。Linux/macOS 数据目录权限为 0700，凭证文件为 0600。

| 接口 | 默认地址 | 用途 |
|---|---|---|
| HTTP | `http://127.0.0.1:7881` | 业务 API、健康检查和监控 |
| gRPC | `http://127.0.0.1:7882` | CLI 与四语言 SDK |
| Web | `http://localhost:3000` | 独立控制台，按下节启动 |

在另一个终端验证：

```sh
curl --fail http://127.0.0.1:7881/ready
./target/debug/gl --token-file ./geoledger-data/admin-credentials.json info
```

就绪检查返回 `{"ok":true}`，CLI 输出版本、后端和请求规格。SQLite 随二进制提供，业务数据由服务端持久化。Ctrl+C 可让服务完成在途请求后退出。

## 连接控制台

按 [Web 本地启动](../web/README.md#本地启动) 构建 TS SDK、安装依赖、配置 `web/.env.local` 并运行开发服务器。访问配置中的 `GL_WEB_ORIGIN`，使用 `admin-credentials.json` 中对应用户的 `token` 登录。

登录后按照 [控制台教程](console.md#添加第一条要素) 创建项目、数据集和工作区，添加要素并发布第一个版本。

## 连接 CLI

```sh
./target/debug/gl --token-file ./geoledger-data/admin-credentials.json projects
printf '%s\n' '{"name":"城市道路"}' | ./target/debug/gl --token-file ./geoledger-data/admin-credentials.json call create_project
```

创建操作返回项目标识、名称和版本等信息。`gl call` 使用业务 JSON，`--file request.json` 读取文件，`--file -` 读取标准输入。

远程访问通过环境中的 `GL_ENDPOINT` 和 `GL_TOKEN` 配置地址与凭证；本机管理员可用 `--token-file` 和 `--subject` 从私有令牌文件选择身份。

## 连接 SDK

选择语言并按 [API 与 SDK](api.md) 安装。SDK 使用相同 gRPC 地址和用户令牌，典型流程为创建项目、创建数据集、创建工作区、保存 GeoJSON、发布和读取。

Python 应用可使用隔离环境安装源码包：

```sh
python3 -m venv .venv
. .venv/bin/activate
python -m pip install ./sdk/python
```

## 使用 PostGIS

管理员准备专用 PostgreSQL 数据库和非超级用户服务账号，在该数据库安装扩展：

```sql
CREATE EXTENSION IF NOT EXISTS postgis;
```

授予服务账号 `public` schema 建表权限。通过秘密管理系统或当前终端环境设置 `GL_DATABASE_URL`，然后启动：

```sh
./target/debug/geoledger-server --storage postgis --data-dir ./geoledger-data
```

启动时初始化空库并校验格式 6；格式 5 的已有库先备份，再运行 `geoledger-server --storage postgis migrate` 升级（见 [格式升级](production.md#格式升级)）。连接默认 `sslmode=verify-full`，私有 CA 通过 `sslrootcert=<PEM 文件>` 指定；本机无 TLS 的开发库使用 `sslmode=disable`，参数说明见 [PostgreSQL TLS](security.md#postgresql-tls)。后端配置选择相应数据库，跨后端迁移使用 `geoledger-server export`、`import` 与 `verify`（见 [备份与恢复](production.md#备份与恢复)）。两种后端沿用相同的 CLI、SDK 与控制台流程。

## 配置团队身份

生成团队凭证并启动服务：

```sh
./target/debug/geoledger-server tokens --out ./team-tokens.json --client-out ./team-credentials.json alice bob
./target/debug/geoledger-server --token-file ./team-tokens.json
```

令牌文件按 create-new 创建，保护已有凭证。`team-tokens.json` 只含摘要，留在服务器；`team-credentials.json` 含明文令牌，分发给对应用户后从服务器删除。过期、吊销和轮换见 [静态令牌](security.md#静态令牌)。用户以各自身份登录；项目创建者获得 owner 角色，可通过控制台或 `set_member` 赋予团队成员权限。身份定义用户，成员关系定义项目访问权限。企业 JWT 接入见 [身份与网络](production.md#身份与网络)。

## 生产构建

```sh
cargo build --release --locked --bins
```

生成 `target/release/geoledger-server` 和 `target/release/gl`，Windows 对应 `.exe`。持久化、网络和恢复配置见 [生产运行](production.md)。

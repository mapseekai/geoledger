# GeoLedger

<img src="web/public/logo.svg" alt="GeoLedger" width="180">

GeoLedger 为道路、地块、监测点等空间要素提供版本控制。团队成员在独立工作区中编辑 GeoJSON，通过属性三方合并、冲突解决和原子发布形成共享版本；历史查询、提交撤销、成员权限与审计帮助追踪和管理数据变化。

当前版本为 `0.3.0-alpha.1`，存储格式为 7。默认 SQLite 随服务端提供，可选 PostGIS；Go、Rust、TypeScript / Node.js 和 Python SDK、远程 `gl` CLI、独立 Web 控制台共用业务服务。

## 获取代码

```sh
git clone git@github.com:mapseekai/geoledger.git
cd geoledger
```

SSH 克隆使用已配置到 GitHub 账号的 SSH 密钥。仓库根目录是 Cargo 工作区，Web 位于 `web/`，四语言 SDK 位于 `sdk/`。以下命令均从仓库根目录执行。

## 环境准备

| 用途 | 工具 |
|---|---|
| 构建服务端和 CLI | [Rust](https://www.rust-lang.org/tools/install) 1.88+，推荐与主 CI 和 Dockerfile 一致的 1.92；[protoc](https://github.com/protocolbuffers/protobuf/releases) 3.21+；C 编译器 |
| Linux/macOS 数据库连接依赖 | pkg-config、OpenSSL 开发库 |
| 仓库检查 | [Python](https://www.python.org/downloads/) 3.10+、Rust 的 rustfmt 与 Clippy |
| Web 控制台和 TS SDK | [Node.js](https://nodejs.org/en/download) 22 与随附 npm |

按操作系统安装和检查工具的步骤见 [快速开始](docs/getting-started.md#环境准备)。Go SDK、PostGIS、容器和浏览器测试工具按所选任务安装。

`Cargo.lock` 与两个 `package-lock.json` 固定依赖解析结果。使用 `cargo --locked` 和 `npm ci`，让本地构建与 CI 使用一致依赖。

## 快速开始

### 本地开发服务

```sh
cargo build --locked --bins
./target/debug/geoledger-server --data-dir ./geoledger-data
```

保持服务端终端运行，在第二个终端检查服务：

```sh
curl --fail http://127.0.0.1:7881/ready
./target/debug/gl --token-file ./geoledger-data/admin-credentials.json info
```

就绪检查返回 `{"ok":true}`；CLI 输出版本、SQLite 后端与格式 6 等服务信息。首次启动直接创建格式 6 的数据库、服务端摘要文件 `tokens.json` 和客户端凭证文件 `admin-credentials.json`。CLI 和控制台使用客户端凭证中的令牌。

### Web 开发服务

按 [Web 本地启动](web/README.md#本地启动) 安装 TS SDK 和控制台依赖、配置会话密钥，随后执行：

```sh
npm --prefix web run dev
```

访问 `http://localhost:3000`，使用管理员分配的令牌登录。控制台提供项目、数据集、工作区、版本历史、访问权限、审计日志与服务信息。第一条要素的操作流程见 [管理控制台](docs/console.md)。

## 构建、测试与部署

构建生产二进制并运行仓库检查：

```sh
cargo build --release --locked --bins
./scripts/check.sh
```

产物为 `target/release/geoledger-server` 和 `target/release/gl`，Windows 使用 `.exe`。Web 的生产构建和检查见 [Web 文档](web/README.md#构建测试与部署)，跨后端及 SDK 验证见 [开发指南](docs/development.md)。

容器部署使用仓库中的 Compose 配置：

```sh
docker compose up --build -d
```

服务端、CLI、SDK 和 Web 控制台按同一发布版本部署。格式 6 的已有数据库可直接沿用；新环境可通过当前格式的备份恢复或逻辑导入导出搬迁完整数据。版本关系见 [当前版本契约](docs/api.md#当前版本契约)，部署配置、持久化、TLS、身份与备份见 [生产运行](docs/production.md)。

## 贡献

通过 [GitHub Issues](https://github.com/mapseekai/geoledger/issues) 报告问题或提出需求，附上版本、操作步骤和期望结果；提交 Pull Request 时说明变更与验证范围。安全漏洞请按 [安全策略](SECURITY.md) 私下报告，不要公开提交 issue；版本变化见 [CHANGELOG](CHANGELOG.md)。

开发和 AI 编码代理遵循 [AGENTS.md](AGENTS.md)，按 [开发指南](docs/development.md#贡献流程) 完成对应检查。项目采用 [MIT 许可证](LICENSE)。

## 文档导航

| 文档 | 内容 |
|---|---|
| [快速开始](docs/getting-started.md) | 获取代码、环境准备、服务启动、CLI 和 SDK 接入 |
| [使用指南](docs/user-guide.md) | 数据模型、协作编辑、发布、冲突、历史与撤销 |
| [管理控制台](docs/console.md) | 页面导航和完整操作示例 |
| [API 与 SDK](docs/api.md) | 四语言接口、HTTP、错误处理与发布恢复 |
| [存储扩展](docs/storage.md) | 分层、事务语义与新后端验收 |
| [生产运行](docs/production.md) | 容器、systemd、TLS、身份、容量与备份 |
| [安全配置](docs/security.md) | 传输加密、令牌轮换与吊销、JWKS、PostgreSQL TLS、控制台会话 |
| [开发指南](docs/development.md) | 构建、生成 SDK、回归测试与贡献流程 |
| [安全策略](SECURITY.md) | 漏洞报告渠道、支持版本与供应链措施 |
| [变更记录](CHANGELOG.md) | 各版本的破坏性变化、安全修复与新增功能 |

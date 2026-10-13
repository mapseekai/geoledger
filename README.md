# GeoLedger

面向空间要素的版本管理与协作编辑服务。

GeoLedger 使用项目、数据集和工作区组织道路、地块、监测点等空间数据。用户在各自的工作区中编辑要素，通过差异审阅、属性级三方合并和发布形成共享修订，并通过历史查询、提交撤销、成员权限与审计记录管理数据变化。

项目提供 Rust 服务端、远程 `gl` 命令行工具、Web 管理控制台，以及 Go、Rust、TypeScript / Node.js 和 Python SDK。存储后端包括 SQLite + SpatiaLite 与 PostGIS，各入口通过统一业务服务执行数据管理和版本操作。

**源码版本：** `0.3.0-alpha.3`

**存储格式：** `14`

**许可证：** MIT

## 核心能力

| 能力 | 说明 |
|---|---|
| 空间数据管理 | 按点、线、面类型组织数据集，配置二维或三维坐标；通过 GeoJSON 编辑和导入要素。 |
| 坐标系管理 | 数据集保存 EPSG 标识或自定义 CRS 定义，默认使用 `EPSG:4326`；导入、存储和发布保留源坐标。 |
| 工作区协作 | 每个用户维护独立草稿，一个工作区可以包含多个数据集的变更；提供分批保存、差异查看、基准更新和发布。 |
| 合并与版本管理 | 对属性执行三方合并，提供冲突字段审阅与解决；发布生成共享修订，并保存发布回执和审计记录。 |
| 历史查询与撤销 | 按修订读取要素，查询提交详情；将指定提交的反向变更保存为工作区，再发布为新修订。 |
| PostGIS 业务表纳管 | 接入已有业务表并建立初始版本；发布事务同步修改源表，同时保留源表坐标系。 |
| GIS 管理界面 | 提供地图、图层树、要素搜索、属性表和 GeoJSON 查看编辑，支持工作区、版本历史、成员权限与审计管理。 |
| 身份与访问管理 | 配置项目成员角色，管理个人令牌；通过静态令牌或 JWT 接入身份认证，配置 TLS 与 mTLS 传输。 |

## 获取代码

```sh
git clone https://github.com/mapseekai/geoledger.git
cd geoledger
```

以下命令均从仓库根目录执行。

## 环境准备

| 使用场景 | 环境与工具 |
|---|---|
| 服务端与 CLI 构建 | Rust 1.88+、Cargo、protoc、C 工具链和 CMake。Linux/macOS 同时准备 pkg-config 与 OpenSSL 开发库。 |
| SQLite 运行环境 | SpatiaLite 原生扩展及其运行依赖。可通过 `GL_SPATIALITE_EXTENSION` 指定扩展路径；项目容器镜像包含该扩展。 |
| Web 控制台与 TypeScript SDK | Node.js 22 与 npm。 |
| Python SDK | Python 3.10+。 |
| 仓库检查 | Python 3、Bash、rustfmt 和 Clippy。 |
| 容器部署 | Docker 与 Docker Compose。 |
| PostGIS 部署 | 已启用 PostGIS 扩展的 PostgreSQL 数据库，以及对应服务账号。 |

各操作系统的工具准备步骤见[快速开始](docs/getting-started.md#环境准备)。Go SDK 的工具链版本以 `sdk/go/go.mod` 为准。

Rust 依赖由 `Cargo.lock` 固定，Web 与 TypeScript SDK 依赖由各自的 `package-lock.json` 固定。构建时使用 `cargo --locked`，安装前端依赖时使用 `npm ci`。

## 主要配置

服务端配置模板位于 `.env.example`，控制台配置模板位于 `web/.env.example`。服务端同时提供命令行参数和环境变量配置。

| 配置项 | 默认值或设置方式 | 用途 |
|---|---|---|
| `GL_STORAGE` | `sqlite` | 选择 `sqlite` 或 `postgis` 存储后端。 |
| `GL_DATA_DIR` | `./geoledger-data` | 服务端数据与身份配置目录。 |
| `GL_HTTP_LISTEN` | `127.0.0.1:7881` | HTTP 接口监听地址。 |
| `GL_GRPC_LISTEN` | `127.0.0.1:7882` | gRPC 接口监听地址。 |
| `GL_DATABASE_URL` | 按数据库部署设置 | PostGIS 连接信息。 |
| `GL_SPATIALITE_EXTENSION` | 按运行时安装路径设置 | SpatiaLite 原生扩展路径。 |
| `GL_WEB_ENDPOINT` | `http://127.0.0.1:7882` | 控制台连接的 gRPC 服务地址。 |
| `GL_WEB_ORIGIN` | `http://localhost:3000` | 控制台的浏览器访问地址。 |
| `GL_WEB_SESSION_SECRET` | 设置至少 32 字符的随机密钥 | 控制台会话配置。 |

`gl` 通过 `GL_ENDPOINT` 配置服务地址，通过 `GL_TOKEN` 或客户端凭证文件配置身份。服务端令牌摘要保存在 `tokens.json`，首次启动生成的客户端凭证保存在 `admin-credentials.json`。

完整配置见[生产运行](docs/production.md)和[安全配置](docs/security.md)。

## 快速开始

### 容器启动

生成会话密钥并启动服务：

```sh
export GL_WEB_SESSION_SECRET="$(openssl rand -base64 48)"
docker compose up --build -d
```

Compose 启动包含业务服务与 Web 控制台的统一镜像，并将数据持久化到 `geoledger-data` 命名卷。

| 入口 | 地址 |
|---|---|
| Web 控制台 | `http://localhost:3000` |
| HTTP API | `http://127.0.0.1:7881` |
| gRPC | `http://127.0.0.1:7882` |

检查服务状态：

```sh
curl --fail http://127.0.0.1:7881/ready
docker compose exec geoledger gl --token-file /data/admin-credentials.json info
```

就绪检查返回 `{"ok":true}`。首次启动在数据卷的 `/data/admin-credentials.json` 中生成客户端凭证，使用其中的用户令牌登录控制台。

### 本地服务启动

准备好 SQLite 的 SpatiaLite 运行环境后，构建并启动服务端：

```sh
cargo build --locked --bins
./target/debug/geoledger-server --data-dir ./geoledger-data
```

保持服务端运行，在另一个终端验证连接：

```sh
curl --fail http://127.0.0.1:7881/ready
./target/debug/gl --token-file ./geoledger-data/admin-credentials.json info
./target/debug/gl --token-file ./geoledger-data/admin-credentials.json projects
```

服务端首次启动会创建 SQLite 数据库、令牌摘要文件与客户端凭证文件。新数据库使用存储格式 14；`gl info` 展示服务版本、存储后端和接口配置。

### Web 本地开发

先构建 TypeScript SDK，再安装控制台依赖：

```sh
npm ci --prefix sdk/ts
npm --prefix sdk/ts run build
npm ci --prefix web
```

创建本地配置并生成会话密钥：

```sh
cp web/.env.example web/.env.local
openssl rand -base64 48
```

将生成的随机值填入 `web/.env.local` 的 `GL_WEB_SESSION_SECRET`，保持 `GL_WEB_ORIGIN` 与浏览器实际访问地址一致，然后启动开发服务：

```sh
npm --prefix web run dev
```

访问 `http://localhost:3000`，使用管理员分配的令牌登录。控制台提供项目、数据集、工作区、版本历史、成员权限、审计和服务信息页面。

## 协作工作流

```text
创建项目与数据集 → 创建工作区 → 保存要素 → 检查差异 → 发布修订 → 查询历史
```

**组织数据。** 项目组织成员、数据集与共享版本。创建数据集时设置几何类型、坐标维度和坐标系；要素使用稳定的字符串 ID 标识，以属性和几何描述业务内容。

**编辑草稿。** 工作区读取基础修订与自身增量。用户通过完整 GeoJSON Feature 保存新增或修改，通过删除操作记录删除意图。分批保存可将多个数据集的变更组织到同一个工作区。

**审阅和合并。** `Diff` 展示草稿差异。发布时，系统将基础版本、当前共享版本和草稿进行三方比较，对不同属性的修改执行自动合并。冲突审阅展示 `base`、`current`、`draft` 和相关字段，用户提交最终结果后，通过 `Rebase` 更新工作区基准。

**发布共享版本。** `Publish` 在同一事务中写入正式要素、历史记录、项目 HEAD、发布回执和审计。SDK 工作区对象维护草稿版本，并为发布生成和保留请求 ID；使用相同请求 ID 与请求内容重试时，可获取已保存的发布结果。

**查询与撤销。** 要素可按修订、要素 ID 或空间范围读取，空间范围采用数据集坐标系。读取共享数据的后续分页时沿用首屏修订。`Restore` 为指定提交创建反向变更工作区，审阅并发布后形成新修订，同时保留原提交及审计记录。

详细操作见[使用指南](docs/user-guide.md)和[管理控制台](docs/console.md)。

## API 与 SDK

HTTP API、gRPC、CLI 和四语言 SDK 共用业务服务。Web 控制台通过 HTTP 接收浏览器操作，其 Node.js 服务端通过 TypeScript SDK 调用 GeoLedger gRPC。

| 接入方式 | 仓库位置 | 用途 |
|---|---|---|
| Rust SDK | `sdk/rust` | Rust 异步应用集成。 |
| Go SDK | `sdk/go` | 使用标准 `context.Context` 的业务调用。 |
| TypeScript / Node.js SDK | `sdk/ts` | Node.js 应用与服务端集成。 |
| Python SDK | `sdk/python` | 数据处理脚本与 Python 应用集成。 |
| CLI | `crates/cli` | 查询资源、执行版本操作和脚本化调用。 |
| 协议定义 | `proto` | gRPC 服务与消息定义。 |

### Python 示例

在开发环境安装 SDK：

```sh
python3 -m venv .venv
. .venv/bin/activate
python -m pip install ./sdk/python
```

在环境变量 `GL_TOKEN` 中配置用户令牌。以下示例创建监测点项目与数据集，保存一个要素，发布后读取对应修订：

```python
import os

from geoledger import Client

with Client("http://127.0.0.1:7882", os.environ["GL_TOKEN"]) as client:
    project = client.create_project("监测点管理")
    dataset = client.create_dataset(
        project.id,
        "monitoring_points",
        "point",
        coordinate_dimension=2,
        crs="EPSG:4326",
    )
    workspace = client.create_workspace(project.id)

    workspace.save(dataset.id, {
        "type": "Feature",
        "id": "station-001",
        "properties": {"name": "一号监测点", "status": "active"},
        "geometry": {"type": "Point", "coordinates": [120.15, 30.27]},
    })

    print(workspace.diff())
    receipt = workspace.publish("新增一号监测点")
    snapshot = client.features(
        project.id,
        dataset.id,
        revision=receipt["revision"],
    )
    print(receipt["revision"], snapshot)
```

完整接口、各语言示例、分页与发布重试说明见 [API 与 SDK](docs/api.md)。

## 项目架构

| 路径 | 职责 |
|---|---|
| `crates/core` | 核心数据结构与合并逻辑。 |
| `crates/engine` | 工作区、要素查询、版本发布、权限与存储事务。 |
| `crates/spatialite` | SpatiaLite 原生运行时集成。 |
| `crates/server` | 服务启动、HTTP 接口、认证、运行配置与运维命令。 |
| `crates/rpc` | gRPC 协议适配。 |
| `crates/cli` | 远程 `gl` 命令行入口。 |
| `sdk` | Go、Rust、TypeScript 和 Python 客户端。 |
| `web` | Web 管理控制台及浏览器接口。 |
| `docs`、`deploy`、`scripts` | 使用文档、部署配置与工程验证脚本。 |

业务引擎通过存储接口组织事务。SQLite 后端结合 SpatiaLite 原生几何与 RTree 索引，PostGIS 后端使用原生 `geometry` 列与 GiST 索引。两种后端沿用相同的项目、工作区、版本与成员协作流程。

存储分层与后端接口见[存储扩展](docs/storage.md)。

## 构建、测试与部署

### 构建与检查

构建生产二进制并执行仓库检查：

```sh
cargo build --release --locked --bins
./scripts/check.sh
```

产物位于 `target/release/geoledger-server` 与 `target/release/gl`，Windows 二进制使用 `.exe` 扩展名。

`check.sh` 包含文档与配置一致性检查、Rust 格式检查、Clippy、工作区测试和备份演练。配置 `GL_TEST_DATABASE_URL` 后，脚本执行对应的 PostGIS 集成验证。CI 还配置了双后端 SDK 往返验证、生成代码一致性检查及 Web 验证。

Web 检查与生产构建：

```sh
npm --prefix web test
npm --prefix web run lint
npm --prefix web run typecheck
npm --prefix web run format:check
npm --prefix web run build
```

验证流程见[开发指南](docs/development.md)和[发布验收](docs/release.md)。

### 生产部署

统一容器镜像包含服务端、CLI、Web 控制台及 SpatiaLite 运行依赖，以非 root 用户运行，并提供健康检查和退出信号处理。业务数据通过 `/data` 持久化。

PostGIS 部署通过 `GL_STORAGE=postgis` 与 `GL_DATABASE_URL` 配置数据库连接，也可在已设置连接环境变量后运行：

```sh
./target/release/geoledger-server --storage postgis --data-dir ./geoledger-data
```

服务端提供 TLS、mTLS、静态令牌、JWT/JWKS 和身份配置热加载。控制台通过 HTTPS 入口和服务端会话管理浏览器访问。部署时统一配置各组件的发布版本、服务地址、身份凭据和数据持久化。

容器、systemd、数据库账号与 HTTPS 配置见[生产运行](docs/production.md)。

## 运行维护

`/health` 提供健康检查，`/ready` 提供就绪检查。服务端输出结构化 JSON 日志，日志级别通过 `RUST_LOG` 配置。

数据运维命令包括 SQLite 在线备份与恢复，以及逻辑导出、导入和校验。备份与导出使用新的目标文件路径，恢复面向新的数据目录，逻辑导入写入新建或空数据库。

```sh
./target/release/geoledger-server --data-dir ./geoledger-data \
  backup --output ./geoledger-backup.sqlite3

./target/release/geoledger-server --data-dir ./geoledger-data \
  export --output ./geoledger-export.jsonl

./target/release/geoledger-server \
  verify --input ./geoledger-export.jsonl --file-only
```

逻辑导出包含结构信息、表级摘要与校验信息，可配合 `import` 和 `verify` 完成业务数据搬迁与校验。备份、恢复及身份配置保存流程见[生产运行](docs/production.md)，令牌管理与传输配置见[安全配置](docs/security.md)。

## 贡献

通过 [GitHub Issues](https://github.com/mapseekai/geoledger/issues) 提交问题或需求，说明使用版本、操作步骤与期望结果。提交 Pull Request 时描述变更内容和验证范围，并按照 [AGENTS.md](AGENTS.md) 与[开发指南](docs/development.md)完成对应检查。

安全相关反馈通过 [SECURITY.md](SECURITY.md) 中的私密渠道提交。版本变化记录于 [CHANGELOG.md](CHANGELOG.md)。

## 文档导航

| 文档 | 内容 |
|---|---|
| [快速开始](docs/getting-started.md) | 环境准备、服务启动、CLI 与 SDK 接入。 |
| [使用指南](docs/user-guide.md) | 数据模型、工作区编辑、合并发布、历史与撤销。 |
| [管理控制台](docs/console.md) | 页面导航、地图工作台与协作操作。 |
| [API 与 SDK](docs/api.md) | HTTP、四语言 SDK、分页与发布恢复。 |
| [存储扩展](docs/storage.md) | 存储分层、事务接口与后端验证。 |
| [生产运行](docs/production.md) | 部署、持久化、数据库配置与备份恢复。 |
| [安全配置](docs/security.md) | TLS、令牌、JWT/JWKS 与控制台会话。 |
| [开发指南](docs/development.md) | 构建、SDK 生成、测试与贡献流程。 |
| [发布验收](docs/release.md) | 交付检查、恢复演练与负载验证。 |
| [变更记录](CHANGELOG.md) | 版本更新与功能变化。 |

## 许可证

GeoLedger 采用 [MIT 许可证](LICENSE)。

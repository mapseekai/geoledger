# GeoLedger

[中文](README.md) · [English](README.en.md)

GeoLedger 是空间要素的版本管理与协作编辑服务。团队用项目组织数据集，在各自的工作区中编辑 GeoJSON 要素，审阅差异、解决冲突，再发布为共享修订。历史查询、提交撤销、成员角色和审计记录贯穿这套工作流。

仓库包含 Rust 服务端 `geoledger-server`、远程命令行工具 `gl`、Web 控制台，以及 Rust、Go、TypeScript / Node.js 和 Python SDK。服务端管理 SQLite + SpatiaLite 或 PostGIS 中的数据，HTTP 与 gRPC 入口调用同一个业务引擎。

当前源码版本为 `0.3.0-alpha.6`，存储格式为 16，分别定义于 [Cargo.toml](Cargo.toml) 和 [engine](crates/engine/src/lib.rs)。

## 核心能力

| 能力 | 使用方式 |
|---|---|
| 数据集与坐标系 | 按点、线、面组织要素，设置二维或三维坐标以及 EPSG 标识或自定义 CRS；默认坐标系为 `EPSG:4326`。保存与发布保留源坐标和 GeoJSON 数值。 |
| 工作区与差异审阅 | 一个工作区可编辑多个数据集，分批保存新增、修改和删除，并查看要素差异及数据集变更统计。草稿版本用于并发编辑校验。 |
| 三方合并与发布 | 比较基础修订、当前共享修订和草稿，合并独立属性修改，按字段审阅冲突。发布在同一事务中写入要素、历史、修订、回执和审计。 |
| 发布结果恢复 | SDK 保留发布请求 ID 和原始请求内容；连接中断后通过原请求重试确认结果。控制台提供发布恢复流程。 |
| 查询与撤销 | 按修订、要素 ID、空间范围和分页游标查询；为指定提交生成反向变更工作区，经审阅后发布为新修订。 |
| PostGIS 表纳管 | 将已有业务表接入数据集并建立初始版本，发布事务同步原表，保留源表坐标系。 |
| GIS 控制台 | 地图、图层树、要素搜索、属性表、GeoJSON 编辑与上传、工作区版本图、冲突审阅和手机布局。地图预览按需转换到 WGS 84。 |
| 身份与访问 | 用户管理、项目成员角色、个人 API Key、数据库会话，以及 HTTP/gRPC 的 TLS 与 mTLS 配置。 |

## 获取代码

```sh
git clone https://github.com/mapseekai/geoledger.git
cd geoledger
```

下文的 shell 命令从仓库根目录执行。

## 环境准备

| 场景 | 工具与依赖 |
|---|---|
| Rust 构建 | Rust 1.88+、Cargo、protoc、C 工具链和 CMake；Linux/macOS 还需 pkg-config 与 OpenSSL 开发库。CI 的常规 Rust 检查使用 1.92.0。 |
| SQLite 服务 | SpatiaLite 原生扩展及其依赖；`GL_SPATIALITE_EXTENSION` 可指定扩展路径。仓库 Dockerfile 包含对应运行环境。 |
| PostGIS 服务 | 启用 PostGIS 扩展的 PostgreSQL 数据库及服务账号。 |
| Web 与 TypeScript SDK | Node.js 22 与 npm。 |
| Go SDK | Go 1.27.2+，见 [go.mod](sdk/go/go.mod)。 |
| Python SDK | Python 3.10+，见 [pyproject.toml](sdk/python/pyproject.toml)。 |
| 仓库检查 | Python 3.11+、Bash、rustfmt、Clippy；浏览器验证使用 Python Playwright 和可启用沙箱的 Chromium/Chrome。 |
| 容器运行 | Docker、Docker Compose，以及用于生成会话密钥的 OpenSSL。 |

Linux、macOS 和 Windows 的准备步骤见[快速开始](docs/getting-started.md#环境准备)。Rust 使用 `Cargo.lock`，Web 与 TypeScript SDK 使用各自的 `package-lock.json`；对应安装命令为 `cargo --locked` 与 `npm ci`。

## 主要配置

服务端选项见 `.env.example` 与 `geoledger-server --help`，控制台选项见 `web/.env.example`。将所需服务端变量载入进程环境；Compose 使用 `.env` 进行变量替换，Next.js 读取 `web/.env.local`。

| 变量 | 默认值或设置方式 | 用途 |
|---|---|---|
| `GL_STORAGE` | `sqlite` | 选择 `sqlite` 或 `postgis`。 |
| `GL_DATA_DIR` | `./geoledger-data` | 服务端数据目录。 |
| `GL_HTTP_LISTEN` | `127.0.0.1:7881` | HTTP 监听地址。 |
| `GL_GRPC_LISTEN` | `127.0.0.1:7882` | gRPC 监听地址。 |
| `GL_DATABASE_URL` | PostGIS 部署时设置 | 数据库连接及 TLS 参数。 |
| `GL_SPATIALITE_EXTENSION` | 按安装路径设置 | SpatiaLite 扩展文件。 |
| `GL_ADMIN_USERNAME` | `admin` | 初始化管理员名称。 |
| `GL_ADMIN_PASSWORD` | 可在初始化前设置 | 初始密码；省略时生成随机密码并写入私有文件。 |
| `GL_WEB_ENDPOINT` | `http://127.0.0.1:7881` | 控制台连接的私有 HTTP 服务。 |
| `GL_WEB_ORIGIN` | `http://localhost:3000` | 浏览器实际访问的完整 origin。 |
| `GL_WEB_SESSION_SECRET` | 至少 32 字符的随机值 | 加密控制台会话 Cookie。 |
| `GL_ENDPOINT` | `http://127.0.0.1:7882` | `gl` 连接的 gRPC 服务。 |

CLI 凭据可选择 `GL_API_KEY` 或私有文件 `GL_API_KEY_FILE` / `--api-key-file`。TLS 证书、私有 CA、请求预算、限流和停机参数见[安全配置](docs/security.md)与[生产运行](docs/production.md)。

## 快速开始

### 容器启动

```sh
export GL_WEB_SESSION_SECRET="$(openssl rand -base64 48)"
docker compose up --build -d
curl --fail http://127.0.0.1:7881/ready
```

[compose.yaml](compose.yaml) 构建并启动包含服务端、CLI 和控制台的镜像，将数据保存在 `geoledger-data` 命名卷的 `/data` 中。就绪检查成功时返回 `{"ok":true}`。

| 入口 | 本地地址 |
|---|---|
| Web 控制台 | `http://localhost:3000` |
| HTTP API | `http://127.0.0.1:7881` |
| gRPC | `http://127.0.0.1:7882` |

首次启动时，随机管理员凭据保存在 `/data/initial-admin-<id>.json`。在自己的终端中读取该初始化文件：

```sh
docker compose exec geoledger sh -c 'cat /data/initial-admin-*.json'
```

使用其中的用户名和密码登录控制台，完成初始密码修改。在“API Key”页面创建个人密钥，保存在自己的私有凭据文件中，供 CLI 或 SDK 使用。

### 本地服务与控制台

准备好 SpatiaLite 后，构建二进制并在独立终端启动服务端：

```sh
cargo build --locked --bins
./target/debug/geoledger-server --data-dir ./geoledger-data
```

服务端创建 SQLite 数据库及初始管理员，并在日志中给出私有初始化凭据文件的路径。保持服务运行，在另一个终端准备控制台：

```sh
npm ci --prefix sdk/ts
npm --prefix sdk/ts run build
npm ci --prefix web
cp web/.env.example web/.env.local
openssl rand -base64 48
```

把生成的随机值填入 `web/.env.local` 的 `GL_WEB_SESSION_SECRET`，按上表设置服务地址和浏览器 origin，然后运行：

```sh
npm --prefix web run dev
```

访问 `http://localhost:3000`，用初始管理员凭据登录并修改密码，随后创建 API Key。将 Key 保存为 `./api-key` 纯文本文件；在 Unix 上设置私有权限后验证 CLI：

```sh
chmod 600 ./api-key
curl --fail http://127.0.0.1:7881/ready
./target/debug/gl --api-key-file ./api-key info
./target/debug/gl --api-key-file ./api-key projects
```

`info` 输出协议、存储后端和请求规格。详细启动步骤见[快速开始](docs/getting-started.md)，界面操作见[控制台教程](docs/console.md)。

## 协作工作流

1. **组织数据。** 创建项目和数据集，选择几何类型、坐标维度与 CRS。要素通过稳定的字符串 ID 标识。
2. **编辑草稿。** 创建工作区，保存完整 GeoJSON Feature 或显式删除操作。查询草稿时读取基础修订与工作区增量。
3. **审阅差异。** 查看 `diff` 和变更统计。共享版本推进时，通过 `base`、`current`、`draft` 对比审阅冲突，提交解决结果并更新基准。
4. **发布修订。** 发布请求携带工作区版本和请求 ID。事务提交后返回回执；重试沿用同一请求 ID 与请求内容。
5. **读取历史。** 按修订查询数据与提交详情。共享数据分页沿用首屏修订，空间范围使用数据集 CRS。
6. **撤销提交。** `restore` 将指定提交的反向变更写入新工作区，审阅后发布新的共享修订。

数据模型和合并规则见[使用指南](docs/user-guide.md)，分页和重试契约见 [API 与 SDK](docs/api.md)。

`diff`、`commit`、`history`、`audit` 使用服务端页级游标确认当前快照的结束；`history` 返回 `commits` 页对象，详见 [分页契约](docs/api.md#差异提交历史与审计分页)。

## API 与 SDK

HTTP 业务接口位于 `/api/v1`，四语言 SDK 与 CLI 通过 `geoledger.v1` gRPC 协议接入。SDK 使用用户 API Key，工作区对象维护草稿版本和待确认的发布请求。

| 接入方式 | 源码与接口 |
|---|---|
| Rust | [sdk/rust](sdk/rust)，异步业务客户端。 |
| Go | [sdk/go](sdk/go)，带 `context.Context` 的业务调用。 |
| TypeScript / Node.js | [sdk/ts](sdk/ts)，Promise 接口与精确 JSON 处理。 |
| Python | [sdk/python](sdk/python)，业务对象与上下文管理器。 |
| CLI | [crates/cli](crates/cli)，`info`、`projects`、`members`、`history` 和通用 `call`。 |
| gRPC 协议 | [geoledger.proto](proto/geoledger/v1/geoledger.proto)。 |

### Python 示例

安装仓库中的 SDK，并使用前文创建的 `./api-key` 文件：

```sh
python3 -m venv .venv
. .venv/bin/activate
python -m pip install ./sdk/python
```

以下代码创建数据集、保存监测点、发布并读回该修订：

```python
from pathlib import Path

from geoledger import Client

api_key = Path("api-key").read_text().strip()
with Client("http://127.0.0.1:7882", api_key) as client:
    project = client.create_project("Monitoring")
    dataset = client.create_dataset(
        project.id, "stations", "point",
        coordinate_dimension=2, crs="EPSG:4326",
    )
    workspace = client.create_workspace(project.id)
    workspace.save(dataset.id, {
        "type": "Feature",
        "id": "station-001",
        "properties": {"name": "Station 1", "sequence": 9007199254740993},
        "geometry": {"type": "Point", "coordinates": [120.15, 30.27]},
    })
    print(workspace.diff())
    receipt = workspace.publish("Add station-001")
    snapshot = client.features(
        project.id, dataset.id, revision=receipt["revision"],
    )
    print(receipt["revision"], snapshot)
```

结果包含发布修订和要素集合，`sequence` 保留整数值。各语言安装方式、认证、业务错误和发布恢复示例见 [API 与 SDK](docs/api.md)。

## 项目架构

```text
Browser -> Next.js BFF -> private Session HTTP -> engine Application
HTTP API / gRPC -> server adapters -----------> engine Application
CLI / SDKs -> gRPC ---------------------------> engine Application
                                                      |
                                     StorageBackend / RepositoryTransaction
                                                      |
                                      SQLite + SpatiaLite / PostGIS
```

浏览器会话使用加密 HttpOnly Cookie 中的随机 Session ID，身份与有效期由数据库管理。业务引擎通过存储事务接口执行权限检查、草稿编辑与版本发布。

| 路径 | 职责 |
|---|---|
| `crates/core` | 数据模型与三方合并。 |
| `crates/engine` | 身份、授权、工作区、历史、发布及存储适配。 |
| `crates/spatialite` | SpatiaLite 原生连接与几何运行时。 |
| `crates/server`、`crates/rpc` | HTTP/gRPC、TLS、服务配置与数据运维命令。 |
| `crates/cli`、`sdk` | 远程命令行及四语言客户端。 |
| `web` | Next.js 控制台、浏览器接口和地图工作台。 |
| `deploy`、`scripts` | 部署模板、验证脚本与发布门禁。 |

SQLite 使用 WAL、SpatiaLite 几何和 RTree 空间索引；PostGIS 使用 `geometry` 列和 GiST 索引。存储事务边界与后端一致性要求见[存储扩展](docs/storage.md)。

## 构建与验证

```sh
cargo build --release --locked --bins
./scripts/check.sh
```

产物位于 `target/release/geoledger-server` 与 `target/release/gl`，Windows 使用 `.exe` 后缀。`check.sh` 需要 Python 3 和 ast-grep 0.51.0（`npm install --global @ast-grep/cli@0.51.0`），先验证三语言测试布局，再执行文档与配置检查、rustfmt、Clippy、引擎与工作区测试、备份恢复演练；设置 `GL_TEST_DATABASE_URL` 可启用隔离 `geoledger_test` 数据库上的 PostGIS 用例。

Web 检查与生产构建：

```sh
npm --prefix web test
npm --prefix web run lint
npm --prefix web run typecheck
npm --prefix web run format:check
npm --prefix web run build
```

[开发指南](docs/development.md) 提供四语言 SDK 联调和真实浏览器验证命令。控制台 CI 的两个浏览器套件共用沙箱配置，Ubuntu 24.04 runner 使用系统 Chrome。依赖门禁 `./scripts/audit.sh` 检查 Rust、npm、Go 和 Python 公告。

alpha.6 的发布准备、本机实测和生产验收边界见 [alpha.6 验收记录](docs/release-alpha6-validation-2026-10-18.md)。早期工作区修复保留在 [2026-10-15 验证记录](docs/ci-validation-2026-10-15.md)。

## 部署与运行维护

[Dockerfile](Dockerfile) 将业务服务、CLI、Next.js standalone 控制台及 SpatiaLite 运行依赖放入统一镜像，以 UID 10001 运行。Compose 持久化 `/data`，容器入口统一处理服务生命周期和健康检查。

PostGIS 部署设置 `GL_STORAGE=postgis`、`GL_DATABASE_URL` 和服务账号。连接配置默认校验证书与主机名，可使用私有 CA；现有业务表通过 SDK 的 `register_table` / `registerTable` / `RegisterTable` 接入。部署模板、systemd 和 HTTPS 网关见[生产运行](docs/production.md)。

| 运维接口 | 行为 |
|---|---|
| `/health` | 返回进程健康状态和服务版本。 |
| `/ready` | 检查服务接单状态、可用执行容量和存储结构。 |
| `/metrics` | 向已认证用户提供 Prometheus 指标。 |
| JSON 日志 | 通过 `RUST_LOG` 设置日志级别，记录请求与运行诊断。 |

数据运维包括 SQLite 在线备份、恢复，以及逻辑导出、导入和校验。下列命令使用新的输出文件：

```sh
./target/release/geoledger-server --data-dir ./geoledger-data \
  backup --output ./geoledger-backup.sqlite3

./target/release/geoledger-server --data-dir ./geoledger-data \
  export --output ./geoledger-export.jsonl

./target/release/geoledger-server \
  verify --input ./geoledger-export.jsonl --file-only
```

恢复使用新的数据目录；逻辑导入写入新建或空数据库，并校验文件结构、表级摘要与完整性校验和。操作步骤与身份数据保存方式见[生产运行](docs/production.md)。

## 贡献

通过 [GitHub Issues](https://github.com/mapseekai/geoledger/issues) 提供版本、复现步骤和期望结果。提交变更前阅读 [AGENTS.md](AGENTS.md)，按[开发指南](docs/development.md)执行相关检查，并说明实际验证范围。

漏洞反馈使用 [SECURITY.md](SECURITY.md) 中的私密报告渠道。版本变更与兼容性说明见 [CHANGELOG.md](CHANGELOG.md) 和[当前版本契约](docs/api.md#当前版本契约)。

## 文档导航

| 文档 | 内容 |
|---|---|
| [快速开始](docs/getting-started.md) | 平台准备、服务启动与客户端接入。 |
| [使用指南](docs/user-guide.md) | 数据模型、协作、合并、历史与撤销。 |
| [控制台教程](docs/console.md) | 地图工作台、资源管理与用户操作。 |
| [API 与 SDK](docs/api.md) | 接口、四语言示例、分页与重试。 |
| [存储扩展](docs/storage.md) | 后端接口、事务语义与一致性测试。 |
| [生产运行](docs/production.md) | 部署、数据库连接与备份恢复。 |
| [安全配置](docs/security.md) | TLS、密码、API Key 和会话。 |
| [开发指南](docs/development.md) | 构建、代码生成与测试。 |
| [发布验收](docs/release.md) | 发布门禁、恢复演练与负载验证。 |

## 许可证

GeoLedger 使用 [MIT 许可证](LICENSE)。控制台组件、字体等第三方许可见 [Web 第三方说明](web/THIRD_PARTY_NOTICES.md)。

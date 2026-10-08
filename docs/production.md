# 生产运行

[项目概览](../README.md) · [存储扩展](storage.md) · [开发验证](development.md)

## 部署形态

一个 `geoledger-server` 进程同时提供 HTTP 管理入口和 gRPC。默认 SQLite 将要素、工作区、版本、成员和审计保存在服务端目录；PostGIS 使用外部 PostgreSQL 服务。客户端仅配置服务地址和凭证。

SQLite 适合单机部署和较轻写入，写事务串行；本地可靠磁盘、持久化目录、WAL/FULL synchronous 是运行前提。活动 SQLite 数据目录使用本地可靠文件系统。PostGIS 适合更高并发写入、集中备份与数据库运维。20 个用户的业务目标需要结合实际几何、编辑量和磁盘性能验收。

源码附 [Dockerfile](../Dockerfile)、[Compose](../compose.yaml)、[Compose 生产参考](../deploy/compose.production.yaml)（PostGIS + TLS 网关）和 [systemd 示例](../deploy/geoledger.service)。容器的 `/data` 必须挂载持久化卷。生产环境在可信网关后暴露端口；示例默认对本机开放。

## 本机与 systemd 部署

从仓库根目录构建并运行生产二进制：

```sh
cargo build --release --locked --bins
./target/release/geoledger-server --data-dir ./geoledger-data
```

使用专用操作系统账号，将二进制和 [systemd 示例](../deploy/geoledger.service) 安装到目标机器，按实际路径、用户和环境文件调整 unit。服务的数据目录和凭证文件由运行账号持有。构建平台的工具准备见 [快速开始](getting-started.md#环境准备)。

## 容器部署

```sh
docker compose up --build -d
```

配置使用命名卷 `geoledger-data` 持久化 `/data`，将 HTTP 和 gRPC 映射到本机 7881/7882。检查就绪与日志：

```sh
curl --fail http://127.0.0.1:7881/ready
docker compose logs geoledger
```

在容器内使用首次启动生成的私有管理员凭证验证业务连接（验证后把该文件交给运维人员并从卷中删除）：

```sh
docker compose exec geoledger gl --token-file /data/admin-credentials.json info
```

控制台组合部署按 [Web 容器运行](../web/README.md#容器运行) 先设置会话密钥，再启用 console profile。`docker compose down` 停止组合服务，命名卷按数据保留策略管理。

### Compose 生产参考

[compose.production.yaml](../deploy/compose.production.yaml) 组合 PostGIS、GeoLedger、控制台和 [nginx TLS 网关](../deploy/gateway/nginx.conf)，只有网关发布 80/443 端口：

- **PostGIS**：使用官方 `postgis/postgis` 镜像（固定 digest），开启 TLS（最低 TLS 1.2），[pg_hba.conf](../deploy/postgis/pg_hba.conf) 只接受 `hostssl` 加 scram-sha-256，明文连接被拒绝。
- **GeoLedger**：以 `sslmode=verify-full` 和私有 CA 连接 PostGIS。根文件系统只读，只有 `/data` 卷可写。启用按身份和按 IP 的限流，并信任网关传入的 `X-Forwarded-For`。
- **控制台**：经私有网络访问 gRPC。
- **网关**：负责 TLS、HSTS、HTTP→HTTPS 跳转、请求 ID 透传和边缘限流。
- **资源与健康检查**：每个服务都设置了 CPU/内存上限和健康检查，并按 `service_healthy` 顺序启动。

```sh
scripts/make-test-certs.sh deploy/tls        # 评估用私有 CA；生产放入同名的正式证书
export GL_DB_PASSWORD=$(openssl rand -hex 32) GL_WEB_SESSION_SECRET=$(openssl rand -base64 48)
docker compose -f deploy/compose.production.yaml up -d --build
```

上线前需要完成以下配置：

- **域名**：把 `nginx.conf` 中的 `api/grpc/console.geoledger.example` 和 `GL_WEB_ORIGIN` 改为实际域名。`deploy/tls/` 中需要有以下文件：
  - `ca.crt`：PostGIS 证书的签发 CA。
  - `gateway.crt`/`gateway.key`：覆盖三个域名的网关证书。
  - `postgis.crt`/`postgis.key`：SAN 为 `postgis` 的数据库证书。
- **管理员凭证**：首次启动生成的管理员凭证在 `geoledger-data` 卷的 `/data/admin-credentials.json`，取出后从卷中删除。
- **身份认证**：生产建议改用 JWT（`GL_JWKS_URL` 等），并设置 `GL_BOOTSTRAP_ADMIN=false`。
- **端口**：`GL_GATEWAY_BIND`、`GL_GATEWAY_HTTPS_PORT`、`GL_GATEWAY_HTTP_PORT` 用于调整网关发布的地址和端口。
- **备份**：PostGIS 按下文 [备份与恢复](#备份与恢复) 使用 `pg_dump` 或 WAL 归档。逻辑导出命令为 `docker compose -f deploy/compose.production.yaml exec geoledger geoledger-server export --output /data/export.jsonl`。

该组合已在 Docker Engine 29 上实测：四个服务均通过健康检查；`gl` 经网关以 TLS 访问 gRPC；HTTPS API 返回 HSTS 和请求 ID；HTTP 跳转到 HTTPS；PostGIS 拒绝明文连接；只读容器内的导出与校验成功。

## 配置

| 环境变量 / 参数 | 默认 / 用途 |
|---|---|
| GL_STORAGE / --storage | sqlite；可选 postgis |
| GL_DATA_DIR / --data-dir | ./geoledger-data |
| GL_DATABASE_URL / --database-url | PostGIS 连接串，只在 postgis 模式使用；TLS 参数见 [安全配置](security.md#postgresql-tls) |
| GL_DATABASE_ALLOW_PLAINTEXT | 允许对非回环主机使用 sslmode=disable，默认 false |
| GL_HTTP_LISTEN / --http | 127.0.0.1:7881 |
| GL_GRPC_LISTEN / --grpc | 127.0.0.1:7882 |
| GL_TLS_CERT / GL_TLS_KEY / GL_TLS_CLIENT_CA | 服务端 TLS 与 mTLS，见 [安全配置](security.md#传输加密) |
| GL_TOKEN_FILE / --token-file | 默认数据目录 tokens.json（SHA-256 摘要格式） |
| GL_BOOTSTRAP_ADMIN | 无令牌文件时生成初始 admin 凭证，默认 true |
| GL_JWKS_FILE / --jwks-file | 可选受信任 JWKS 文件，变更后自动加载 |
| GL_JWKS_URL / --jwks-url | 可选 HTTPS JWKS 地址，按 GL_JWKS_REFRESH_SECS（默认 300）刷新 |
| GL_RELOAD_INTERVAL_SECS | 令牌、JWKS、证书文件检查间隔，默认 10；SIGHUP 立即加载 |
| GL_JWT_ISSUER / --jwt-issuer | 固定 HTTPS issuer |
| GL_JWT_AUDIENCE / --jwt-audience | 固定 audience |
| GL_HEALTH_LISTEN | 可选明文探针监听地址，只提供 /health 与 /ready |
| GL_MAX_CONCURRENCY | HTTP 与 gRPC 共用执行槽，默认 20 |
| GL_REQUEST_TIMEOUT_SECS | 单次操作期限上限，默认 30 |
| GL_DB_POOL_SIZE | PostgreSQL 连接上限，默认 20 |
| GL_DB_STATEMENT_TIMEOUT_SECS / GL_DB_LOCK_TIMEOUT_SECS | PostgreSQL 语句与锁等待期限，默认 30 / 10 |
| GL_RATE_LIMIT_SUBJECT_RPS / _BURST | 每个身份的持续速率与突发量，默认 0（关闭）/ 50 |
| GL_RATE_LIMIT_IP_RPS / _BURST | 每个客户端 IP 的速率与突发量（认证前检查），默认 0（关闭）/ 100 |
| GL_TRUST_FORWARDED_FOR | 以 X-Forwarded-For 最右侧地址作为客户端 IP，仅在可信网关后开启，默认 false |
| GL_SHUTDOWN_DRAIN_SECS | 收到停止信号后保持服务、/ready 返回 503 的秒数，默认 0 |
| GL_SHUTDOWN_TIMEOUT_SECS | 关闭监听器后等待在途请求的上限，默认 30 |
| GL_ADMIN_SUBJECTS / --admin-subjects | 平台管理员 subject，逗号分隔；可管理任意项目的成员、归档与删除，不获得数据读写权限 |
| GL_PROJECT_CREATION / --project-creation | `anyone`（默认）或 `admins`：仅平台管理员可创建项目 |
| GL_MAX_PROJECTS_PER_SUBJECT / --max-projects-per-subject | 每个身份拥有（owner 且未删除）的项目上限，默认 0 表示不限；管理员不受限 |
| GL_DATA_TIMEOUT_SECS | backup、export、import、verify 命令的期限，默认 3600 |
| RUST_LOG | geoledger_server=info,geoledger_engine=info，结构化 JSON 日志 |

启动时校验显式配置，确保使用指定存储和身份文件。新库自动初始化为格式 6；已有库必须匹配当前格式，旧格式的库启动时返回 409 并提示先备份再执行 `geoledger-server migrate`。服务启动从不修改已有库的结构，升级步骤见 [格式升级](#格式升级)。

根目录 [.env.example](../.env.example) 提供当前服务配置模板。本机二进制从进程环境读取变量，部署时通过 shell、systemd EnvironmentFile 或秘密管理系统注入；Compose 从 `.env` 读取控制台 origin 和会话密钥。原生 Web 使用自己的 `web/.env.local`。

## 身份与网络

初次默认启动生成摘要格式的 `tokens.json` 与一次性的 `admin-credentials.json`；多人使用独立凭证与项目成员关系。静态凭证文件或 JWT 二选一。令牌支持过期时间、吊销与热加载，JWKS 支持文件热加载与 HTTPS 定时刷新，详见 [安全配置](security.md)。

公网访问使用服务端 TLS（`GL_TLS_CERT`/`GL_TLS_KEY`，可选 mTLS）或 TLS 网关：HTTP 转发到 HTTP 监听器；gRPC 网关保持 HTTP/2 并转发到 gRPC 监听器，参考 [nginx.conf](../deploy/gateway/nginx.conf) 与 [Caddyfile](../deploy/gateway/Caddyfile)。SDK 的 https 地址启用服务器证书验证；http 地址只用于回环主机，其他主机需要显式开启明文。凭证通过秘密管理注入；浏览器使用加密 HttpOnly 会话 Cookie，代理通过过滤认证头和请求体日志保护凭证。

PostGIS 使用专用非超级用户，只授权独立数据库；管理员安装扩展后让服务账号创建应用表。默认 `sslmode=verify-full`，跨主机数据库连接校验证书链与主机名。每个实例的活跃数据库会话上限为 `GL_DB_POOL_SIZE`（默认 20），数据库和网关连接预算按实例数计算。

## 容量规划

- HTTP 与 RPC 共用 `GL_MAX_CONCURRENCY` 个执行槽（默认 20），耗尽时明确返回繁忙（429）。
- 默认操作期限 `GL_REQUEST_TIMEOUT_SECS`（30 秒）；RPC 可缩短期限。PostgreSQL 单语句与锁等待期限可配置（默认 30 / 10 秒），同时受总期限约束；SQLite 锁等待服从剩余总期限。
- 速率限制按身份和客户端 IP 使用令牌桶，超限返回 429 与 `Retry-After: 1`。部署在网关后时，在网关限速（参考 [nginx.conf](../deploy/gateway/nginx.conf) 的 `limit_req`），或开启 `GL_TRUST_FORWARDED_FOR` 让服务按真实客户端地址限速。
- 请求/响应最多 4 MiB，单个 Feature 最多 16 KiB、256 个属性；空间坐标为 EPSG:4326 XY/XYZ。
- 每批 Save 最多 100 条修改，单工作区最多 1000 个不同要素。百万要素初次导入需要分批、分工作区；客户端为每次发布保留独立请求 ID。
- 列表默认 100、最多 1000 条；响应还有展开内存预算，复杂属性需降低页大小。
- 磁盘预算覆盖原始要素、历史、索引、WAL、审计与备份，按数据增长和保留策略预留空间。

容量验收结合当前存储格式、几何类型、历史深度、RPC 请求模型和目标环境开展，测量吞吐、延迟分位数与磁盘增长。专项测试入口见 [容量验证](development.md#容量验证)。

## 运维与恢复

### 健康检查

`/health` 检查进程，`/ready` 检查数据库格式与连接，失败或关闭中返回 503。gRPC 监听器同时提供标准 [gRPC Health Checking](https://grpc.io/docs/guides/health-checking/) 服务（服务名 `geoledger.v1.GeoLedger`），关闭时切换为 `NOT_SERVING`。启用服务端 TLS 时，可设置 `GL_HEALTH_LISTEN=127.0.0.1:7880` 提供仅含探针的明文监听器。

`geoledger-server probe` 按同一组 `GL_*` 配置访问本机 `/ready`，成功退出码为 0；容器镜像的 `HEALTHCHECK` 与 Compose 健康检查使用该命令。控制台提供 `/api/health` 存活检查。

### 日志与请求关联

日志为单行 JSON。每次业务调用输出一条访问日志（target `geoledger_server::access`），字段包括 `request_id`、`protocol`、`operation`、`status`、`duration_ms`、`subject` 与 `peer`。服务接受网关传入的 `x-request-id`（最长 128 个字符，字母数字与 `-_.:`），否则生成 UUID；同一 ID 写入响应头、gRPC 元数据与错误体 `request_id`。5xx 错误额外输出 `operation failed` 错误日志，`diagnostic` 字段只含安全摘要（PostgreSQL 仅记录 SQLSTATE）。排查用户报告的错误时，用错误体中的 `request_id` 检索日志。控制台服务端把 5xx 失败写为 JSON 行，包含 GeoLedger 的 `requestId`。

### 指标

`/metrics` 需要 Bearer 身份，输出 Prometheus 文本格式：

| 指标 | 说明 |
|---|---|
| `geoledger_operations_total{protocol,operation,status}` | 按协议、操作、状态码统计的调用次数 |
| `geoledger_operation_duration_seconds{protocol,operation}` | 按协议与操作的耗时直方图 |
| `geoledger_requests_total` / `errors_total` / `internal_errors_total` / `conflicts_total` / `busy_total` | 汇总计数 |
| `geoledger_auth_failures_total` | 认证失败次数 |
| `geoledger_rate_limited_total{scope}` | 按 `subject` / `ip` 的限速拒绝次数 |
| `geoledger_in_flight_requests`、`geoledger_execution_slots_available`、`geoledger_execution_slots_capacity` | 并发占用 |
| `geoledger_db_pool_connections{state}`、`geoledger_db_pool_capacity`、`geoledger_db_pool_wait_timeouts_total` | PostgreSQL 连接池（postgis 模式） |
| `geoledger_draining`、`geoledger_build_info{version}` | 关闭状态与版本 |

标签只使用固定的操作名与状态码集合，从不使用身份、项目、要素 ID 或请求 ID。建议告警：`internal_errors_total` 增速、`rate(busy_total)`、`db_pool_wait_timeouts_total` 增长、`/ready` 失败。

### 格式升级

存储格式变化随版本说明公布。升级由运维显式执行，服务启动从不自动迁移：

1. 停止全部服务实例，按下文备份数据库（SQLite 数据目录或 PostgreSQL 一致性备份）。
2. 使用新版本二进制和与服务相同的 `GL_*` 存储配置执行 `geoledger-server migrate --check`，输出当前格式与目标格式，不做修改；有待执行的升级时退出码为 3。
3. 执行 `geoledger-server migrate`。迁移在单个事务内完成，失败时整体回滚，库保持原格式。
4. 启动新版本服务，检查 `/ready`、项目列表与历史查询。

格式 5 → 6 增加成员移除标记、项目状态（active/archived/deleted）和成员索引，已有项目全部为 active、成员全部保留。迁移只向前进行；回退版本时恢复第 1 步的备份。

### 项目与成员治理

项目 owner 可列出成员（`list_members`）、修改角色、移除成员（`remove_member`，成员也可退出项目），但项目必须保留至少一名 owner。owner 可归档项目（`archive_project`）：归档后数据只读，成员管理、恢复与删除仍可执行。删除（`delete_project`）需提交与项目名称一致的 `confirm_name`，项目从所有列表与查询中隐藏，历史与审计保留在库中以备审查。

`GL_ADMIN_SUBJECTS` 指定的平台管理员可对任意项目执行上述成员与生命周期操作（审计记录其 subject），用于离职交接和孤儿项目处理，但不因此获得要素、历史或审计的读取权限。`GL_PROJECT_CREATION=admins` 与 `GL_MAX_PROJECTS_PER_SUBJECT` 限制项目创建，拒绝时返回 403。配额为软上限：PostgreSQL 多实例同时创建时可能短暂超出 1 个。

### 停止与发布确认

停止进程使用 SIGTERM / Ctrl+C：服务先把 `/ready` 与 gRPC 健康状态切换为不可用并保持 `GL_SHUTDOWN_DRAIN_SECS` 秒，然后关闭监听器，在 `GL_SHUTDOWN_TIMEOUT_SECS` 内等待在途请求完成；超时后记录警告并以非零状态退出。编排系统的终止宽限期应大于两者之和（Compose 示例为 60 秒）。发布结果未知时使用原 request_id 和原内容确认，以服务端保存的发布收据确定提交结果。

### 备份与恢复

| 命令 | 作用 |
|---|---|
| `geoledger-server backup --output <文件>` | SQLite 在线一致备份（`VACUUM INTO`，服务可继续写入），完成后做完整性与格式校验并输出各表行数与摘要 |
| `geoledger-server restore --input <文件>` | 校验 SQLite 备份后安装为 `--data-dir` 的数据库；目标库已存在时拒绝 |
| `geoledger-server export --output <文件>` | 逻辑导出（JSON 行），覆盖全部业务表：项目、成员、数据集、工作区与草稿、提交、历史、发布收据和审计；在一致快照中读取，服务可继续运行 |
| `geoledger-server import --input <文件>` | 导入到新库或空库；整个文件校验通过才提交，损坏或截断时库保持不变 |
| `geoledger-server verify --input <文件>` | 校验导出文件结构、表摘要与校验和，并与当前库逐表比较；不一致时退出码为 4。`--file-only` 只校验文件 |

所有命令使用与服务相同的 `GL_STORAGE`、`GL_DATA_DIR`、`GL_DATABASE_URL` 配置，期限由 `GL_DATA_TIMEOUT_SECS`（默认 3600）设置。输出文件以 0600 新建且从不覆盖已有文件；导出包含全部业务数据和发布请求内容，按凭证同等级别加密保存。导出文件的表摘要与后端无关，SQLite 与 PostgreSQL 之间可以互相导入，用于更换后端或迁移主机；摘要用于发现损坏和不完整的复制，文件真实性通过存储与传输的访问控制保证。

**SQLite。** 按 RPO 定时执行 `backup`，把备份文件复制到另一台主机或对象存储，并保留多个版本。systemd 部署可使用 [geoledger-backup.service](../deploy/geoledger-backup.service) 与 [geoledger-backup.timer](../deploy/geoledger-backup.timer)：每小时写入带时间戳的新文件并校验，删除本机超过 `GL_BACKUP_KEEP_DAYS`（默认 14）天的副本；先执行 `install -d -o geoledger -g geoledger -m 0700 /var/backups/geoledger`，再 `systemctl enable --now geoledger-backup.timer`。RPO 等于备份间隔；恢复步骤：

1. 停止服务，将原数据目录改名保留。
2. 使用新的空数据目录执行 `geoledger-server --data-dir <新目录> restore --input <备份文件>`，并放回 `tokens.json` 等凭证文件（凭证不在数据库备份中）。
3. 启动服务，检查 `/ready`、项目 HEAD、审计条数和最近的发布收据。

RTO 主要是文件复制时间加一次启动，通常为分钟级。容器部署在运行中的容器内执行同样的命令，例如 `docker compose exec geoledger geoledger-server backup --output /data/backup.sqlite3`，再把文件复制出数据卷。

**PostgreSQL。** 使用 `pg_dump -Fc` 定时逻辑备份或 `pg_basebackup` 加 WAL 归档实现 PITR（RPO 可到秒级），恢复后检查 `/ready`、项目 HEAD 与审计条数；恢复点与某次导出一致时（例如停机窗口内先导出再备份），可用 `verify` 逐表比对。`backup`/`restore` 命令只处理 SQLite；`export`/`import` 适用于两种后端。

**恢复演练。** [scripts/backup-drill.sh](../scripts/backup-drill.sh) 在一次性服务上完成：写入数据 → 服务运行中备份与导出 → 恢复到新目录并启动 → 校验 HEAD、审计和精确数字 → 用导出校验恢复结果 → 导入新库并校验；设置 `GL_DRILL_DATABASE_URL`（空 PostgreSQL 库）时同时导入 PostgreSQL。该脚本是 `scripts/check.sh` 的一部分，生产环境按季度在独立主机上用真实备份重复同样步骤并记录耗时，作为 RTO 的实测依据。

凭证、JWKS 与 TLS 私钥独立加密备份；数据库与凭证文件的访问权限都应纳入恢复流程。

## 独立 Web 管理服务

管理页面独立部署为 [Next.js 控制台](../web/README.md#生产运行)，由其 Node.js
服务端调用 TS SDK。数据库仅由 GeoLedger 服务访问。Web 的公开入口使用 HTTPS，
固定 `GL_WEB_ORIGIN`；随机 `GL_WEB_SESSION_SECRET` 只在运行时配置，多实例共享。
反向代理设置请求体、连接数与登录速率限制。Web 和服务端的内部连接应限制在
可信网络，跨网络使用 gRPC TLS。`compose.yaml` 的 console profile 提供本地组合启动。

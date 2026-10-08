# 生产运行

[项目概览](../README.md) · [存储扩展](storage.md) · [开发验证](development.md)

## 部署形态

一个 `geoledger-server` 进程同时提供 HTTP 管理入口和 gRPC。默认 SQLite 将要素、工作区、版本、成员和审计保存在服务端目录；PostGIS 使用外部 PostgreSQL 服务。客户端仅配置服务地址和凭证。

SQLite 适合单机部署和较轻写入，写事务串行；本地可靠磁盘、持久化目录、WAL/FULL synchronous 是运行前提。活动 SQLite 数据目录使用本地可靠文件系统。PostGIS 适合更高并发写入、集中备份与数据库运维。20 个用户的业务目标需要结合实际几何、编辑量和磁盘性能验收。

源码附 [Dockerfile](../Dockerfile)、[Compose](../compose.yaml) 和 [systemd 示例](../deploy/geoledger.service)。容器的 `/data` 必须挂载持久化卷。生产环境在可信网关后暴露端口；示例默认对本机开放。

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

在容器内使用生成的私有管理员文件验证业务连接：

```sh
docker compose exec geoledger gl --token-file /data/tokens.json info
```

控制台组合部署按 [Web 容器运行](../web/README.md#容器运行) 先设置会话密钥，再启用 console profile。`docker compose down` 停止组合服务，命名卷按数据保留策略管理。

## 配置

| 环境变量 / 参数 | 默认 / 用途 |
|---|---|
| GL_STORAGE / --storage | sqlite；可选 postgis |
| GL_DATA_DIR / --data-dir | ./geoledger-data |
| GL_DATABASE_URL / --database-url | PostGIS 连接串，只在 postgis 模式使用 |
| GL_HTTP_LISTEN / --http | 127.0.0.1:7881 |
| GL_GRPC_LISTEN / --grpc | 127.0.0.1:7882 |
| GL_TOKEN_FILE / --token-file | 默认数据目录 tokens.json |
| GL_JWKS_FILE / --jwks-file | 可选受信任 JWKS 文件 |
| GL_JWT_ISSUER / --jwt-issuer | 固定 HTTPS issuer |
| GL_JWT_AUDIENCE / --jwt-audience | 固定 audience |
| RUST_LOG | geoledger_server=info，结构化 JSON 日志 |

启动时校验显式配置，确保使用指定存储和身份文件。新库自动初始化，已有库必须匹配格式 5。旧版本数据应导出并导入到新库，部署前核对数据及历史保留要求。

根目录 [.env.example](../.env.example) 提供当前服务配置模板。本机二进制从进程环境读取变量，部署时通过 shell、systemd EnvironmentFile 或秘密管理系统注入；Compose 从 `.env` 读取控制台 origin 和会话密钥。原生 Web 使用自己的 `web/.env.local`。

## 身份与网络

初次默认启动生成 admin 凭证；多人使用独立凭证与项目成员关系。静态凭证文件或 JWT 二选一。JWT 接受受信任 JWKS 中的 RS256 密钥，校验签名、issuer、audience、exp、可选 nbf 和 subject。文件由运维分发，密钥轮换后重启服务；在线 JWKS 自动刷新与集中即时撤销需由身份网关提供。

公网访问通过 TLS 网关：HTTP 转发到 HTTP 监听器；gRPC 网关保持 HTTP/2 并转发到 gRPC 监听器。SDK 的 https 地址启用服务器证书验证，http 地址显式使用明文。凭证通过秘密管理注入；浏览器使用加密 HttpOnly 会话 Cookie，代理通过过滤认证头和请求体日志保护凭证。

PostGIS 使用专用非超级用户，只授权独立数据库；管理员安装扩展后让服务账号创建应用表。跨主机数据库连接使用证书校验。每个实例最多 20 条活跃数据库会话，数据库和网关连接预算按实例数计算。

## 容量规划

- HTTP 与 RPC 共用 20 个执行槽，耗尽时明确返回繁忙。
- 默认操作期限 30 秒；RPC 可缩短期限。PostgreSQL 单语句 30 秒、锁等待 10 秒，同时受总期限约束；SQLite 锁等待服从剩余总期限。
- 请求/响应最多 4 MiB，单个 Feature 最多 16 KiB、256 个属性；空间坐标为 EPSG:4326 XY/XYZ。
- 每批 Save 最多 100 条修改，单工作区最多 1000 个不同要素。百万要素初次导入需要分批、分工作区；客户端为每次发布保留独立请求 ID。
- 列表默认 100、最多 1000 条；响应还有展开内存预算，复杂属性需降低页大小。
- 磁盘预算覆盖原始要素、历史、索引、WAL、审计与备份，按数据增长和保留策略预留空间。

容量验收结合当前存储格式、几何类型、历史深度、RPC 请求模型和目标环境开展，测量吞吐、延迟分位数与磁盘增长。专项测试入口见 [容量验证](development.md#容量验证)。

## 运维与恢复

`/health` 检查进程，`/ready` 检查数据库格式与连接，失败返回 503。`/metrics` 需要 Bearer 身份，输出应用调用次数、错误、冲突、繁忙、耗时直方图与可用执行槽。指标采用聚合标签，应用调用计数从认证及请求体读取完成后进入应用层时开始。

停止进程使用 SIGTERM / Ctrl+C，让 HTTP、RPC 完成在途请求后退出。发布结果未知时使用原 request_id 和原内容确认，以服务端保存的发布收据确定提交结果。

SQLite 简单可靠的备份流程是停止服务后备份整个数据目录，再恢复服务；在线备份使用 SQLite backup API 或经过验证的备份工具，备份工具应保证数据库与 WAL 的一致性。恢复到独立目录后执行健康检查、历史查询和发布重试验证。

PostGIS 使用 PostgreSQL 一致性备份，按目标 RPO 配置 WAL 归档/PITR。定期在独立数据库执行恢复演练。凭证和 JWKS 独立加密备份；数据库与凭证文件的访问权限都应纳入恢复流程。

## 独立 Web 管理服务

管理页面独立部署为 [Next.js 控制台](../web/README.md#生产运行)，由其 Node.js
服务端调用 TS SDK。数据库仅由 GeoLedger 服务访问。Web 的公开入口使用 HTTPS，
固定 `GL_WEB_ORIGIN`；随机 `GL_WEB_SESSION_SECRET` 只在运行时配置，多实例共享。
反向代理设置请求体、连接数与登录速率限制。Web 和服务端的内部连接应限制在
可信网络，跨网络使用 gRPC TLS。`compose.yaml` 的 console profile 提供本地组合启动。

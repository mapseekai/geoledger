# 变更记录

本文件遵循 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/) 的结构，版本号遵循 [语义化版本](https://semver.org/lang/zh-CN/)（`0.x` 预发布期间，次版本可能包含破坏性变化，均列在 **Breaking**）。版本契约见 [当前版本契约](docs/api.md#当前版本契约)，漏洞报告见 [安全策略](SECURITY.md)。

发布时把 `Unreleased` 改为版本号与日期（与 `Cargo.toml`、`sdk/ts/package.json`、`sdk/python/pyproject.toml` 一致），发布流程从对应段落生成 Release 说明。

## [0.3.0-alpha.1] - 2026-10-09

`0.3.0-alpha.1`：服务端统一为 `geoledger-server` + 远程 `gl` CLI + 四语言 SDK + 独立 Web 控制台，并补齐生产运行所需的安全、运维与数据生命周期能力。

### Breaking

- **存储格式 6**：新库直接创建当前结构，已有库按当前格式校验。格式变化使用独立新库和当前接口导入源业务要素。
- **PostgreSQL 连接默认 `sslmode=verify-full`**，拒绝 `prefer`/`allow`；无 TLS 的数据库须显式 `sslmode=disable`，非回环主机还需 `GL_DATABASE_ALLOW_PLAINTEXT=true`。
- **令牌文件改为哈希存储**：首次启动生成 `tokens.json`（摘要）与 `admin-credentials.json`（明文，交给运维）；客户端使用后者。服务端只读取摘要格式，客户端使用独立凭证文件。
- **SDK 与 `gl` 拒绝以明文 `http://` 向非回环主机发送凭证**，需要 `GL_ALLOW_INSECURE_TRANSPORT=true` 或对应的 `allowInsecure` 选项显式放行。
- **控制台会话改为服务端存储**：重启控制台会使用户重新登录，多实例部署需会话粘滞。
- 第三方存储后端：`RepositoryTransaction` 新增必需方法（成员移除、项目状态等）。
- Go SDK 要求 Go 1.25+（grpc-go 1.83）。

### Security

- 服务端 HTTP/gRPC 可选 TLS 与 mTLS，证书热加载；非回环地址明文监听时告警。
- 令牌 SHA-256 存储、过期与禁用、无需重启的轮换与吊销（文件变化或 SIGHUP）；JWKS URL 定时刷新并保留最后一次有效密钥。
- 控制台注销立即吊销服务端会话，空闲与绝对超时。
- 按身份与按 IP 的令牌桶限流；可配置并发、请求期限、连接池与数据库超时。
- Go SDK 升级 grpc-go 1.83.2，修复 GO-2026-6348、GO-2026-6061。
- 依赖安全门禁 `scripts/audit.sh`（cargo audit、npm audit、govulncheck、pip-audit）与 Dependabot；Dockerfile 基础镜像固定 digest。
- `.gitignore`/`.dockerignore` 排除 `*credentials*.json`。

### Added

- 成员管理：`list_members`、`remove_member`（软删除，审计）；项目归档与删除；平台管理员 `GL_ADMIN_SUBJECTS`、项目创建策略与每身份配额。
- 数据生命周期：`geoledger-server export`/`import`/`verify`（与后端无关的逻辑导出，SQLite ⇄ PostgreSQL 互通），SQLite 在线 `backup` 与 `restore`，`scripts/backup-drill.sh` 恢复演练，systemd 备份 timer。
- 可观测性：带 request ID 的结构化访问日志、按 operation 的指标、连接池指标、gRPC 标准健康检查、`GL_HEALTH_LISTEN`、`geoledger-server probe` 与容器 HEALTHCHECK、控制台 `/api/health`。
- 有界优雅停机：先让 `/ready` 返回 503，再在 `GL_SHUTDOWN_TIMEOUT_SECS` 内排空。
- 部署参考：nginx/Caddy TLS 网关、`deploy/compose.production.yaml`（TLS PostGIS + 网关 + 控制台）、`scripts/make-test-certs.sh`。
- 发布 workflow（多架构镜像与签名、二进制、SBOM、SDK 发布）；CI 增加依赖审计与协议生成一致性检查，删除失效的 Windows workflow；`SECURITY.md`、本变更记录与当前版本契约。
- Prometheus 告警规则与抓取示例 `deploy/monitoring/`（含 promtool 规则测试）。
- 测试：编解码与几何的 cargo-fuzz 目标、多实例并发发布、超时/429/鉴权失败路径集成用例、k6 负载脚本 `scripts/load-test.sh`，以及每日运行它们的 nightly workflow。

### Changed

- 控制台 BFF 按凭证复用 gRPC 客户端（LRU 256、空闲 5 分钟），不再每个请求重新握手；注销与认证失败时关闭对应连接。
- 错误文案与内部暂存表去掉旧名称 “Center”（错误码不变）。
- 运维文档补充数据增长监控、保留与冷归档建议。
- `.env.example` 按实际参数重写，`scripts/check-env.py` 校验一致性。
- 运维子命令的日志写到 stderr，`export --output -` 可直接用于管道。
- CI 改用官方 `postgis/postgis` 镜像。

### Removed

- 失效的 `center-windows.yml`、`windows-cli.yml` workflow（Windows 打包并入发布流程）。

## [center-v0.2.0-alpha.1] - 2026-09-29

GeoLedger Center Windows x64 协作测试版，见 [GitHub Release](https://github.com/mapseekai/geoledger/releases/tag/center-v0.2.0-alpha.1)。

## [0.1.0-alpha.1] - 2026-09-29

GeoLedger Windows x64 测试版，见 [GitHub Release](https://github.com/mapseekai/geoledger/releases/tag/v0.1.0-alpha.1)。

[Unreleased]: https://github.com/mapseekai/geoledger/compare/v0.1.0-alpha.1...HEAD
[center-v0.2.0-alpha.1]: https://github.com/mapseekai/geoledger/releases/tag/center-v0.2.0-alpha.1
[0.1.0-alpha.1]: https://github.com/mapseekai/geoledger/releases/tag/v0.1.0-alpha.1

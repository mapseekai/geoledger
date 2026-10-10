# 变更记录

本文件遵循 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/) 的结构，版本号遵循 [语义化版本](https://semver.org/lang/zh-CN/)（`0.x` 预发布期间，次版本可能包含破坏性变化，均列在 **Breaking**）。版本契约见 [当前版本契约](docs/api.md#当前版本契约)，漏洞报告见 [安全策略](SECURITY.md)。

发布时把 `Unreleased` 改为版本号与日期（与 `Cargo.toml`、`sdk/ts/package.json`、`sdk/python/pyproject.toml` 一致），发布流程从对应段落生成 Release 说明。

## Unreleased

### Breaking

- 存储格式 11：PostGIS 业务表绑定与事务内变更跟踪、草稿冲突选择部分索引；新库直接初始化当前结构。
- 当前原生空间存储：SQLite 接入 SpatiaLite 原生几何与 RTree；PostGIS 使用 geometry 列与 GiST。保留精确几何快照、XYZ 坐标和拓扑警告，逻辑导入重建原生索引。
- 数据集创建增加不可变坐标维度（二维／三维），默认二维；Web 与四种 SDK 支持选择，写入严格匹配维度。

- 数据集创建必须指定 `point`、`line` 或 `polygon` 几何类型，类型创建后不可更改；新库直接初始化当前格式。


### Added

- PostGIS 已有业务表纳管：自动建立初始版本，发布事务同步原表，支持原表约束回滚和解绑保留。

- 项目与数据集重命名、删除的界面和四语言 SDK 接口；删除数据集时保留共享工作区与版本中其他数据集的记录，清理变空的关联记录。

- 工作区版本来源图：发布主线、当前基线分叉及真实发布来源；历史接口与四语言 SDK 返回来源工作区和发布基线。
- 可视化冲突审阅：双侧字段对比、逐字段或整要素选择、结果预览与可选 GeoJSON 编辑，支持分批解决后更新基准。

### Changed

- GeoJSON 上传位于创建数据集入口，自动识别几何类型和维度，导入新工作区后预览与发布。

- 要素查询以 `application/geo+json` 返回 `FeatureCollection`、返回数量和翻页链接，新增项目作用域 GET items 路由；控制台保留大整数精度及分页版本一致性。

- 工作区取消累计 1000 个修改的限制，仍采用分批保存和原子发布；自相交等拓扑问题按原坐标保存，通过 Save 警告、控制台提示和审计记录说明。
- GeoJSON 文件在 Web Worker 中解析，减少大文件导入对界面交互的阻塞，并避免无特殊属性时逐字符重建整份 JSON。

- 工作区采用从上到下旧到新的 GitGraph 版本图：连续发布沿主线延续，仅真实分叉绘制彩色旁支，缩小圆形节点并移除主干首尾装饰；列出提交说明、提交人和时间。未提交草稿以空心节点区分，提供“版本图 / 表格”切换并保留原表格操作。

### Fixed

- 工作区和版本历史统计改为数据库按数据集聚合，避免每 20 条串行拉取完整 GeoJSON；同时合并界面中正在进行的重复统计请求。

- GeoJSON 上传识别显式声明的 EPSG:3857 并转换为 WGS84，解决米制行政边界被经纬度校验拒绝的问题；上传前校验数据集几何类型。

- TypeScript SDK 与控制台保留合法 `__proto__` 属性及精确数字；解决操作使用同一冲突快照的项目和草稿版本，拒绝过期选择。
- 修复侧栏直接进入数据集、工作区和版本历史时缺少项目上下文；同一会话恢复当前项目，显式错误项目不回退到其他项目。
- 版本图的节点与来源、合入关系使用同一轨道布局，支线版本不再重复落在主线上；移除图内工作区选择器、独立 HEAD 行及重复分隔线。

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

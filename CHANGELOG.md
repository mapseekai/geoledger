# 变更记录

本文件遵循 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/) 的结构，版本号遵循 [语义化版本](https://semver.org/lang/zh-CN/)（`0.x` 预发布期间，次版本可能包含破坏性变化，均列在 **Breaking**）。版本契约见 [当前版本契约](docs/api.md#当前版本契约)，漏洞报告见 [安全策略](SECURITY.md)。

发布时把 `Unreleased` 改为版本号与日期（与 `Cargo.toml`、`sdk/ts/package.json`、`sdk/python/pyproject.toml` 一致），发布流程从对应段落生成 Release 说明。

## [Unreleased]

### Breaking

- 存储格式升级为 18。每个自建数据集拥有独立正式数据表和空间索引，初始化使用新数据库；旧格式数据库及其备份独立保留。

### Changed

- SQLite/SpatiaLite 与 PostGIS 均按业务数据集分表。同类型的多个数据集也各有独立表。当前要素只存于正式表或纳管原表；历史仅保存被替换或删除的旧正文，提交明细仅保存要素 ID 与增删标记。发布、删除和逻辑导入在同一事务内维护数据表；注册 PostGIS 业务表继续保留原表。

### Fixed

- PostGIS 已有表纳管只记录表绑定、初始修订号和保护触发器，并为文本排序分页建立 ID 索引；不再逐行复制原表属性和几何。后续发布仅对涉及的 ID 存储旧版本与版本号，未被修改的源表行仍只存在原表中。
- 控制台在创建数据集和发布版本后刷新当前项目状态，数据集地图和工作区发布入口均立即显示最新版本号。
- 发布草稿创建后直接使用 API 返回的 ID，避免列表还未包含新草稿时要求人工重跑；继续核对已有标签 SHA、草稿身份与已上传资产摘要。

## [0.3.0-alpha.6] - 2026-10-18

### Breaking

- 表注册 SDK 方法改为 Rust/Python `register_table`、Go `RegisterTable`、TypeScript `registerTable`；原 `track_table` / `TrackTable` / `trackTable` 调用需直接改名，不保留旧别名。Web 私有动作同步改为 `registerTable`，部署后刷新旧页面。
- 已有表基准脚本的注册耗时字段由 `track_seconds` 改为 `registration_seconds`；读取新报告的工具需同步更新。
- `history` 的 HTTP 响应与四语言 SDK 返回值改为 `commits` 和页级 `next_after`（TS/Web 为 `nextAfter`）的页对象。调用方须随服务端更新；存储格式仍为 16，无需数据迁移。
- 存储后端事务接口增加 diff/commit/history/audit 的仅键后续探测操作；自定义适配器需要实现相同快照及内存预算语义。

### Changed

- 表注册继续共用 `CreateDataset` / `create_dataset` 与 `postgis_table` 协议，字段编号和 CLI 调用不变。存储格式、既有绑定、备份和历史记录无需迁移；新注册提交说明使用 `Register table`。

### Added

- PostGIS 发布门禁实际执行 TLS-only/SCRAM、正确 CA、错误 CA、错误主机名及禁止明文降级验收；受限角色、原生恢复和恢复后发布使用 verify-full。
- 补齐约 20 人单实例的生产放行清单，覆盖固定镜像 digest、初始改密与 API Key、真实数据负载、异机恢复和告警通知；目标环境结果独立记录。

### Fixed

- 镜像构建不保留未参与编译的 Oxlint/tsgolint 工具，避免其内嵌旧 Go 标准库进入构建环境；CI 继续执行完整 lint，构建与运行环境扫描门禁保持启用。
- 备用 Caddy 网关连接统一镜像中的控制台服务，首次部署指引区分初始化密码和 CLI API Key。
- diff/commit/history/audit 在同一读取快照探测后续记录，仅真实中间页返回页级游标；整页终页与短终页均明确结束，Web 使用服务端游标。history/audit 后续新增记录可通过最后观察位置重新查询。

- PostGIS 发布通过完整要素键定位待关闭的历史记录，避免项目数据分布不均及统计信息滞后时扫描整个项目的历史数据。

### Validation

- 发布准备、本机测量和目标环境待补项目见 [alpha.6 验收记录](docs/release-alpha6-validation-2026-10-18.md)。最终发布由同候选 SHA 的完整 CI、性能、双架构扫描及原生镜像拉取验收控制。

## [0.3.0-alpha.4] - 2026-10-15

### Breaking

- 存储格式为 16，替代上一发布版本的格式 14；需要初始化新数据库，不提供旧格式原地升级或旧备份导入。升级前独立保留旧数据库、备份及对应版本程序。
- 身份认证统一为稳定用户、Argon2id 密码、个人 API Key 和数据库会话；CLI 与四语言 SDK 使用 API Key，不保留旧 token/JWT 认证兼容。控制台升级后重新登录。

### Changed

- 控制台统一使用 shadcn 组件，完善弹窗焦点恢复、未保存编辑保护、账户与冲突界面及手机布局。
- SQLite 发布减少历史扫描；PostGIS 合并复用暂存表并刷新统计信息，补充事务隔离与查询工作量回归。
- 重写中英文项目说明，统一运行示例与能力说明，保留独立验证记录。

### Fixed

- Windows CLI 条件编译消除未使用变量，同时保留 Unix 私有权限、设备号和 inode 检查。
- 两个控制台浏览器套件共用开启沙箱的启动器，Ubuntu 24.04 CI 使用系统 Chrome。
- rustls 更新至 0.23.46，修复 RUSTSEC-2026-0346 至 RUSTSEC-2026-0349。

### Validation

- 版本准备前的提交 `20060f5aa3c7a1f2892686d96c4518448c720511` 已通过 [完整 CI](https://github.com/mapseekai/geoledger/actions/runs/38699676301) 的 7 个 job；该结果不替代新候选提交的发布验收。
- alpha.4 本地 release 二进制通过 20 并发、300 秒 PostGIS HTTP 负载，publish p95 为 514 毫秒；双后端各完成 180 组增量样本及 300 秒 soak，SQLite/PostGIS 分别完成 1,124/751 个循环且零错误，原阈值和完整性检查通过。该记录基于版本准备工作区，不替代最终提交的同 SHA 远端验收。
- 增加仅源码 prerelease 与 Docker Hub 的独立手动流程，要求同 SHA CI、原阈值性能验证、双架构扫描、签名和原生 digest 拉取验收；既有完整 SDK 发布流程保持独立。最终镜像与公开 Release 以该流程实际结果为准。

## [0.3.0-alpha.3] - 2026-10-12

### Breaking

- 存储格式升级为 14：数据集持久保存不可变 CRS，空间索引使用原始坐标。需要初始化新数据库，仅接受格式 14；格式 13 的旧库及备份须独立保留。
- 导入保留源坐标，不再自动把 EPSG:3857 转换为 EPSG:4326。空间查询范围须使用数据集坐标系。

### Added

- 原生数据集支持 EPSG 标识及自定义 CRS 定义，默认 EPSG:4326；协议、四语言 SDK、备份与逻辑导入导出统一携带 CRS。
- PostGIS 表纳管保存源表 CRS，并验证行级 SRID 一致性；发布保留源表坐标系。
- 控制台支持输入坐标系、读取文件声明；仅地图预览转换到 WGS 84，原始要素 JSON 与坐标保持不变。缺少可解析定义时提示无法预览，数据仍可保存。
- 非 WGS 84 要素集合以 application/json 返回并携带 CRS，避免错误声明 RFC 7946 地理坐标语义。

### Validation

- SQLite/PostGIS 回归、四语言 SDK 冒烟、Web 71 项测试及生产构建通过；浏览器验证 EPSG:3857 导入、地图预览、发布和原坐标读回。

## [0.3.0-alpha.2] - 2026-10-11

- 服务端与 Web 控制台合并为一个 Linux 镜像，统一启动、健康检查和优雅退出，Compose 与发布扫描同步使用统一产物。

### Breaking

- Go SDK 最低工具链提高到 1.27.2，更新 x/net 等网络依赖的安全补丁；依赖审计要求全部扫描器就绪。

- SDK 表纳管方法统一为 Rust/Python `track_table`、Go `TrackTable`、TypeScript `trackTable`，替换原有 `attach` 命名；Web 同步使用 `trackTable` 动作，不保留旧别名。原生数据集创建继续使用 `create_dataset` 系列方法。

- 存储格式 13：统一成员凭据、签发回执、全局身份审计、事务草稿计数和版本化冲突索引；新库直接初始化当前结构，当前格式支持备份恢复与逻辑搬迁。
- 当前原生空间存储：SQLite 接入 SpatiaLite 原生几何与 RTree；PostGIS 使用 geometry 列与 GiST。保留精确几何快照、XYZ 坐标和拓扑警告，逻辑导入重建原生索引。
- 数据集创建增加不可变坐标维度（二维／三维），默认二维；Web 与四种 SDK 支持选择，写入严格匹配维度。

- 数据集创建必须指定 `point`、`line` 或 `polygon` 几何类型，类型创建后不可更改；新库直接初始化当前格式。


### Added

- 平台管理员可通过控制台或四语言 SDK 签发、撤销及重新签发成员个人令牌；首次成功响应展示明文，重试通过回执确认。
- 成员权限使用标准弹窗，项目状态独立展示归档、恢复与名称确认删除。
- SDK 使用统一固定编译器生成；发布草稿在 SDK 分发和安装验证后公开，强制校验两份镜像清单与源提交一致。
- 生产模板区分初始化管理员与受限运行账号；CI 验证跨 schema 纳管、原生备份恢复、成员凭据恢复与浏览器发布。
- 双后端持续增量负载覆盖 10/100/500 条增删改、发布重放、读回与延迟分位数；首次发布与收据重放分别统计并验收。

- 发布前同一 SHA 的完整质量门禁、双架构 OCI 漏洞扫描及准确 digest 发布；已发布镜像每日复审。
- Windows 校验和锁定的 SpatiaLite 安装器，以及发布二进制的空间写入／查询／重启冒烟检查。

- PostGIS 已有业务表纳管：自动建立初始版本，发布事务同步原表，支持原表约束回滚和解绑保留。

- 项目与数据集重命名、删除的界面和四语言 SDK 接口；删除数据集时保留共享工作区与版本中其他数据集的记录，清理变空的关联记录。

- 工作区版本来源图：发布主线、当前基线分叉及真实发布来源；历史接口与四语言 SDK 返回来源工作区和发布基线。
- 可视化冲突审阅：双侧字段对比、逐字段或整要素选择、结果预览与可选 GeoJSON 编辑，支持分批解决后更新基准。

### Changed

- 完善传输准入、数据库清理、锁顺序、发布恢复、分页与历史查询、构建隔离及可观测性；验证范围见 [发布验收](docs/release.md)。
- 草稿保存合并基线与冲突状态读取，按行数和字节上限批量写入，失败时整个请求回滚。
- PostGIS 独立工作区可并发保存；普通源表按批次同步发布，带业务触发器的源表保留逐行执行顺序；合并临时表按数据集与要素键索引，减少批量原表校验的重复扫描。
- 请求和响应新增可配置的单消息与并发编码字节预算；默认单消息 64 MiB，总接收/响应预留各 256 MiB。gRPC 在解码前认证，慢响应额度随流生命周期释放。
- 超时数据库连接在回滚确认或后端身份消失前保留名额；空闲断线在业务事务之前检测，取消与超时独立归因。 PostGIS 按序发送超时设置与查询，减少逐条操作的网络等待，并持续递减操作预算。

- GeoJSON 上传位于创建数据集入口，自动识别几何类型和维度，导入新工作区后预览与发布。

- 要素查询以 `application/geo+json` 返回 `FeatureCollection`、返回数量和翻页链接，新增项目作用域 GET items 路由；控制台保留大整数精度及分页版本一致性。

- 工作区取消累计 1000 个修改的限制，仍采用分批保存和原子发布；自相交等拓扑问题按原坐标保存，通过 Save 警告、控制台提示和审计记录说明。
- GeoJSON 文件在 Web Worker 中解析，减少大文件导入对界面交互的阻塞，并避免无特殊属性时逐字符重建整份 JSON。

- 工作区采用从上到下旧到新的 GitGraph 版本图：连续发布沿主线延续，仅真实分叉绘制彩色旁支，缩小圆形节点并移除主干首尾装饰；列出提交说明、提交人和时间。未提交草稿以空心节点区分，提供“版本图 / 表格”切换并保留原表格操作。

### Fixed

- PostGIS 已提交发布收据的重放在权限校验后直接返回，避免等待项目发布写锁；首次发布仍在锁内再次校验原请求。
- npm 包声明源码仓库与包目录，满足 provenance 的来源匹配要求。

- 发布重跑校验已有 Release 资产、SDK 包内容和 Go 标签，补齐中断的分发；干净环境安装后的 SDK 执行真实增删改、历史查询与发布重试。

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

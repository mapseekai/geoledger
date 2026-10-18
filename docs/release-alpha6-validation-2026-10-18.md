# alpha.6 发布准备与验收（2026-10-18）

## 范围与候选

目标仍为约 20 人团队、单实例、PostGIS＋nginx TLS 网关。发布源码及统一 Linux AMD64/ARM64 镜像；目标生产机待准备，正式机器的真实数据、异机恢复和告警通知留待部署验收。

版本为 `0.3.0-alpha.6`（Python `0.3.0a6`），存储格式仍为 16。[alpha.5 准备记录](release-alpha5-validation-2026-10-18.md) 保留基础功能测量及该候选被镜像门禁拦截的结果。alpha.5 标签不移动，不将先前候选的通过结果替代本候选的同 SHA 门禁。

## 构建环境修复

alpha.5 扫描在两个前端依赖目录的 `@oxlint-tsgolint/*/tsgolint` 中发现 Go 1.26.8 的 CVE-2026-78667、CVE-2026-78669、CVE-2026-97031。它们是 lint 工具的内嵌运行时，扫描结果不能据此称为业务服务存在可利用的网络漏洞。公告依据为 [Go Range 解析公告](https://pkg.go.dev/vuln/GO-2026-6609)、[HTTP/2 公告](https://pkg.go.dev/vuln/GO-2026-6611)、[TLS 公告](https://pkg.go.dev/vuln/GO-2026-6607)。

镜像构建不执行 lint，故在 `npm ci --ignore-scripts` 后完整卸载 Oxlint 与 tsgolint。`--no-save` 保留源码清单和锁文件；CI 继续安装相同版本并执行 lint。参与编译的 TypeScript 仍使用已固定源码、摘要和 Go 1.27.2 的现有重建流程。完整构建环境及运行镜像仍需经过扫描，没有添加忽略项或放宽漏洞门禁。

## 验证与发布依据

本机为 macOS ARM64、Rust 1.92.0、Node.js 22.23.1、npm 10.9.8；Docker 控制台阶段使用固定的 Node.js 22.23.3、npm 12.2.0 和 Go 1.27.2。

| 检查 | 实际状态 |
|---|---|
| 版本及文档 | alpha.6 全包版本、Python 等价版本和文档链接检查通过。 |
| 全仓检查 | 指定 Homebrew SpatiaLite 执行 scripts/check.sh 通过，包含脚本/发布失败注入、fmt、Clippy、工作区测试和 SQLite 在线备份/恢复/导入导出；HEAD r3、11 条审计保留。 |
| 修改后的 Docker 控制台阶段 | 实际 ARM64 构建通过：SDK 与 Web 生产编译成功；断言 Oxlint/tsgolint 及其原生包移除，npm 保留的作用域目录为空；四份 package.json/package-lock.json 摘要与源码一致，standalone 服务产物存在。 |
| 本机 npm 审计 | sdk/ts 与 web 的全部依赖 npm audit 均报告 0 漏洞，使用 npm registry 公告端点。 |
| 本机四语言审计 | scripts/audit.sh 按 fail-closed 行为因缺少 cargo-audit 退出；cargo-audit、govulncheck、pip-audit 均未安装，本机 Rust/Go/Python 审计记为阻塞。最终候选 CI 需按既有固定工具版本实际执行 RustSec、npm、Go 漏洞库及 Python 公告审计，不能以 npm 结果替代。 |

本机 GL_TEST_DATABASE_URL 未设置，常规 PostGIS 专项和两套浏览器套件由最终候选 CI 执行；本机 Docker 控制台生产编译不作为浏览器验收结果。

本机准备验证和最终同 SHA 发布门禁分别记录。最终候选提交、镜像 digest、双架构原生拉取和性能证据以本版本 GitHub Release 资产及 [Source prerelease and Docker Hub](../.github/workflows/source-docker-release.yml) 实际运行结果为准。

## 最终发布与隔离演练

[alpha.6 Release](https://github.com/mapseekai/geoledger/releases/tag/v0.3.0-alpha.6) 已公开为 prerelease，源码与标签均为 `ffa3ecb50305016862f6b34ea648c724eb9937b7`。镜像为 `docker.io/zwishing/geoledger@sha256:94d937c75a298d31b983e5e5bb236468cb231c797b982d71f59b57616bb8b035`，覆盖 Linux AMD64/ARM64。

[运行 39109842294](https://github.com/mapseekai/geoledger/actions/runs/39109842294) 已成功完成：Linux、Windows 原生启动、MSRV、两套 Web 浏览器用例、四语言依赖审计、官方 PostGIS 的真实 TLS/恢复/浏览器演练、性能、双架构构建与运行扫描、合并扫描、签名及匿名原生拉取 SDK 验收。四份原生扫描报告均为 0 条阻塞级 HIGH/CRITICAL 公告。

该候选的 PostGIS HTTP 负载以 20 并发持续 5 分钟，54,038 次请求，失败率和繁忙率为 0；features/save/publish p95 约 239/140/480 毫秒。10,000 点要素的 SQLite/PostGIS 增量测试各以 20 workers 运行 300 秒，分别完成 1,467/738 个循环，错误列表为空，原 p95 门限通过。官方 PostGIS 小夹具恢复约 1.14 秒；这些结果均不作为真实业务或生产 RTO 承诺。

本机按最终镜像 digest 启动独立生产 Compose 项目，只绑定回环端口，实际通过网关证书校验、PostGIS verify-full、随机初始化密码登录、强制改密、Secure/HttpOnly Cookie、API Key、经 TLS 网关的 gRPC CLI、精确 JSON、原请求发布重试、重启后要素/历史/审计/Session 保留与退出登录。临时容器、网络和数据卷已清理。此演练为原生 Linux ARM64 容器，数据库使用 `imresamu/postgis@sha256:088210a8556029c7b3dafa7c721f40ba4dfda70e1f3ec19a4d53b943902fd33f`；官方固定 PostGIS 镜像的结果来自上述 Linux CI，不将替代镜像冒充官方镜像。

Release 资产保留统一镜像清单、两个架构的 acceptance JSON 与 performance.zip；没有发布独立 SDK 包或二进制安装包。

## 收尾流程后续修补

首次收尾已创建草稿，但紧接着的列表查询未返回该草稿，因此流程安全停止。只重跑收尾后成功公开，镜像与受检源码保持原值。后续代码直接使用 [创建 Release API](https://docs.github.com/en/rest/releases/releases#create-a-release) 返回的草稿 ID，继续校验标签 SHA、草稿状态/身份及已存在资产摘要；预先存在的草稿仍通过认证分页查询恢复。

失败注入已复现原错误，并验证列表仍为空时可用创建响应继续上传，同时拒绝错误标签、非草稿和错误 prerelease 状态。79 项供应链/发布回归、18 项源码发布回归及 scripts/check.sh 全仓检查通过，SQLite 恢复保留 HEAD r3 和 11 条审计。该流程修补单独提交供后续发布使用，不移动 alpha.6 标签、不覆盖已发布镜像。

[生产放行清单](production.md#约-20-人单实例的上线放行清单) 中正式 DNS/证书、真实数据 20 并发持续运行、异机备份恢复、告警接收与负责人签署均未在本轮目标机器上验证。

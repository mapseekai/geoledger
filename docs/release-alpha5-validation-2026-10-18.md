# alpha.5 发布准备与验收（2026-10-18）

## 范围

目标为约 20 人团队、单实例、PostGIS＋nginx TLS 网关。本轮沿用源码和统一 Linux AMD64/ARM64 镜像的预发布流程，不发布 npm/PyPI 包或独立二进制。目标生产机器由用户确认待准备，真实数据负载、异机恢复和告警通知不能在本轮记为生产通过。

版本为 `0.3.0-alpha.5`（Python `0.3.0a5`），存储格式仍为 16。alpha.4 的格式 16 数据无需格式迁移；表注册方法和 history 返回页对象的调用方需按 CHANGELOG 同步更新。

## 实现

- PostGIS 发布演练使用生产的 hostssl/SCRAM 配置、正确的私钥权限及 `verify-full` 连接，执行真实 PostgreSQL TLS 测试，然后验证受限角色、原生备份恢复、恢复后发布和浏览器表注册。
- TLS 测试覆盖默认 verify-full、verify-ca、require、错误 CA、错误主机名、TLS-only 拒绝明文、明文服务器不被静默接受。错误主机名另以 verify-ca 成功作为可达性和信任链对照。
- 发布策略回归拒绝跳过 PostGIS 演练；正式发布仍由同一候选 SHA 的完整 CI、性能、双架构扫描与原生镜像验收控制。
- 修正备用 Caddy 控制台 upstream 为统一服务；此处做配置对照检查，发布路径仍使用 nginx。
- 生产手册补齐初始化密码、强制改密、API Key 和 CLI 文件验证；明确私有 Session HTTP、固定镜像 digest，以及目标机器的单实例放行清单。

## 本机测量

以下针对提交前的 alpha.5 准备工作区，不替代最终候选提交的远端门禁。环境为 macOS ARM64、Rust 1.92.0、Node.js 22.23.1。

| 检查 | 实际状态 |
|---|---|
| 包版本、CHANGELOG 与锁文件 | `check-release-version.py --tag v0.3.0-alpha.5` 通过 |
| 发布失败注入 | 78 项通过 |
| 源码/镜像预发布回归 | 18 项通过 |
| Compose 配置 | 2 项通过 |
| Actions YAML | actionlint 通过 |
| Web | 90 项测试、lint、typecheck、format:check、生产构建通过 |
| TLS 真实连接 | 2 项通过，0 ignored；包括正确 CA、错误 CA/主机名和明文拒绝 |
| PostGIS 恢复与业务 | 通过：TLS＋受限角色、原生备份恢复、恢复后历史/审计/成员凭据、继续发布、1/100/500 条源表 CRUD 和解绑；小夹具恢复约 31.69 秒，不作为生产 RTO 承诺。 |
| 全仓检查与 SQLite 恢复 | 通过：指定 Homebrew SpatiaLite 执行 scripts/check.sh，包含布局、脚本回归、fmt、Clippy、工作区测试和 SQLite 在线备份/恢复/导入导出（HEAD r3、11 条审计保留）。本机 GL_TEST_DATABASE_URL 未设置，常规 PostGIS 专项由最终候选同 SHA CI 执行。 |

本机未重复真实浏览器套件，最终候选由远端 CI 执行两套 Web 浏览器用例及 TLS PostGIS 浏览器演练；CI 只保留非敏感 result.json 作为 postgis-release-acceptance 产物。

本机正常生产构建在沙箱外完成；沙箱内的 Turbopack 编译停滞后已停止。浏览器 sandbox 设置保持开启。

仓库默认官方 PostGIS 固定镜像没有 ARM64 manifest；本机模拟 AMD64 初始化过慢而停止，实际功能测量使用已缓存的 ARM64 `imresamu/postgis@sha256:088210a8556029c7b3dafa7c721f40ba4dfda70e1f3ec19a4d53b943902fd33f`。正式 Linux CI 仍使用仓库固定的官方镜像，必须取得自身运行的通过结果。本机未把模拟架构或替代镜像结果当作官方 Linux 镜像验收。

## 最终发布与生产放行

alpha.5 未公开发布。[候选运行 39106869628](https://github.com/mapseekai/geoledger/actions/runs/39106869628) 对提交 `21988205b727d91f86de2f584b2c8161d1e35489` 的完整 CI、性能和官方 PostGIS TLS/恢复/浏览器演练通过，但构建环境中未使用的 tsgolint 内嵌 Go 1.26.8，触发 3 项 HIGH 公告（两套依赖目录共 6 条）。发布被门禁拦截，没有推送新镜像或公开 Release；已固定的标签保留原提交。

修复与最终候选见 [alpha.6 验收记录](release-alpha6-validation-2026-10-18.md)。本页保留 alpha.5 的测量来源，不作为 alpha.6 同 SHA 证据。

[生产放行清单](production.md#约-20-人单实例的上线放行清单) 中的正式 DNS/证书、真实数据 20 并发持续运行、异机备份恢复、告警接收人与负责人签署，留待目标机器准备后逐项执行。隔离库和测试凭据均不用于生产。

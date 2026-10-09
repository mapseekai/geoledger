# 安全策略

[项目概览](README.md) · [安全配置](docs/security.md) · [变更记录](CHANGELOG.md)

## 支持的版本

GeoLedger 处于 `0.x` 预发布阶段。安全修复发布在最新的预发布版本及其后续版本中；升级说明写在 [CHANGELOG](CHANGELOG.md) 的对应版本段落。

| 版本 | 安全修复 |
|---|---|
| 最新 `0.3.x` 预发布 | 提供 |
| 更早的预发布（`v0.1.0-alpha.1`、`center-v0.2.0-alpha.1`） | 请升级到最新版本 |

## 报告漏洞

请**私下**报告，避免在修复发布前公开细节：

1. 首选 GitHub 私密漏洞报告：在仓库 **Security → Report a vulnerability** 提交（维护者需在仓库设置中启用 *Private vulnerability reporting*）。
2. 该入口不可用时，开一个只写“请求私密安全联系方式”的 issue，**不要**包含漏洞细节，维护者会提供私密渠道。

报告请包含：受影响的版本或 commit、部署方式（SQLite / PostGIS、容器或 systemd、是否经过网关）、复现步骤或概念验证、影响评估，以及希望如何署名。日志与样例数据请先脱敏，不要附带真实令牌、私钥或数据库文件。

处理目标：

| 阶段 | 目标时间 |
|---|---|
| 确认收到 | 3 个工作日内 |
| 初步评估与严重程度 | 10 个工作日内 |
| 修复发布 | 严重和高危问题优先发布补丁版本；其余随下一个版本发布 |

修复发布后通过 GitHub Security Advisory 公开（必要时申请 CVE），并在 CHANGELOG 的 **Security** 小节记录。报告者同意时在公告中致谢。

## 范围

范围内：`geoledger-server`（HTTP、gRPC、认证、存储与迁移、导出导入与备份命令）、`gl` CLI、四种 SDK、`web/` 控制台及其 BFF、仓库内的部署参考（Dockerfile、Compose、systemd、nginx/Caddy 网关）。

以下情况通常不作为漏洞处理，但欢迎以普通 issue 报告加固建议：

- 需要已经拥有服务器文件系统或数据库超级用户权限的攻击。
- 违反 [安全配置](docs/security.md) 的部署，例如在公网上明文监听、关闭 PostgreSQL TLS 校验、把 `GL_ALLOW_INSECURE_TRANSPORT` 用于非可信网络。
- 依赖库中已公开、且仓库依赖门禁（`scripts/audit.sh`、Dependabot）正在跟踪的公告。

## 供应链

- CI 依赖门禁 [scripts/audit.sh](scripts/audit.sh)：`cargo audit`、`npm audit`、`govulncheck`、`pip-audit`。已接受的公告及理由记录在 [.cargo/audit.toml](.cargo/audit.toml)。
- [Dependabot](.github/dependabot.yml) 每周提交依赖更新；GitHub Actions 固定到 commit SHA，Dockerfile 基础镜像固定 digest。
- 发布产物附 SHA256、SBOM 和构建来源证明，容器镜像使用 cosign 无密钥签名，见 [开发指南](docs/development.md#依赖与发布)。

## 协议与存储契约

`geoledger.v1` 协议、HTTP `/api/v1` 与存储格式的版本规则见 [当前版本契约](docs/api.md#当前版本契约)。

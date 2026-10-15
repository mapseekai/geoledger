# 2026-10-15 CI 修复与文档验证记录

## 范围与基线

本记录区分源码能力、已执行检查与待执行验证。工作区基线为 `409011516edabf576b6f8c75e0c86f096215a5a6`，对应 [原 CI run](https://github.com/mapseekai/geoledger/actions/runs/38686302500)。修复保留在本地 `work` 分支工作区，提交和远端运行另行记录。

本次实现变更限于 CLI 条件编译、两个控制台测试的浏览器配置及 rustls 安全补丁。随后更新根 `AGENTS.md`、重写中文 README、增加对应英文 README，并让文档检查器覆盖两份根 README。性能阈值与 nightly p95 配置保持原样。

## 修复内容

| 项目 | 文件与变化 |
|---|---|
| Windows Clippy | [CLI](../crates/cli/src/main.rs) 的 `opened` 元数据绑定移入 `cfg(unix)` 块。Unix 私有文件权限、设备号和 inode 校验保持原样。 |
| 浏览器沙箱 | [共享启动器](../scripts/browser_test_support.py) 始终设置 `chromium_sandbox=True`，供 [console](../scripts/test-console.py) 和 [shadcn](../scripts/test-console-shadcn.py) 测试调用。`--chromium` 与 `--browser-channel` 互斥；[CI](../.github/workflows/ci.yml) 选用系统 Chrome。 |
| rustls | [Cargo.lock](../Cargo.lock) 仅将 rustls 从 `0.23.45` 更新为 `0.23.46` 及其校验和；既有审计策略保持原样。 |

浏览器方案使用已有系统安装路径及 AppArmor 配置，依据 [Chromium 官方说明](https://chromium.googlesource.com/chromium/src/+/main/docs/security/apparmor-userns-restrictions.md)。执行过程中没有修改 AppArmor、sysctl、浏览器管理策略或沙箱安全选项。

## 执行环境

- Linux x86_64 云开发环境。
- Rust/Cargo 1.92.0；兼容性检查使用 Rust 1.88.0。
- Node.js 22.23.3；Python 3.12.14；Go 1.27.2。
- Python Playwright 1.62.0；系统 Chromium 151.0.7922.173（Debian）。
- cargo-audit 0.22.2、govulncheck 1.8.0、pip-audit 2.9.0。
- 服务、浏览器和 SDK 联调使用独立临时 SQLite 数据库、随机测试账户与私有凭据文件。

命令执行层的隔离最初阻止 Chromium 使用自己的沙箱。在获准的执行环境中启动同一浏览器后，`chrome://sandbox` 确认 PID 命名空间、网络命名空间和 Seccomp-BPF 均开启，并成功访问本地 HTTP 页面。

## 已执行的代码与集成检查

下表记录文档重写之前完成的修复验证；文档阶段保持这些实现文件不变。

| 命令或套件 | 实测结果 |
|---|---|
| `./scripts/check.sh` | 退出 0。文档、环境、容器启动器、Compose 配置、供应链策略回归、Rust 格式、Clippy、引擎与工作区测试，以及备份恢复演练完成。 |
| `cargo clippy --locked --workspace --all-targets -- -D warnings` | 由总检查执行，通过。 |
| `cargo +1.88.0 check --workspace --all-targets --locked` | 通过。 |
| `cargo test --locked -p geoledger-cli` | 2 个测试通过，包括 Unix 私有权限和符号链接检查。 |
| `cargo test --locked -p geoledger-server --test security` | TLS HTTP、TLS gRPC、明文连接拒绝、mTLS 成功和缺少客户端证书拒绝均通过。 |
| `cargo test --locked -p geoledger-server --test resource_lifecycle tls_grpc_admission_preserves_ip_rate_limits` | 通过。 |
| `cargo test --locked -p geoledger-server --lib tls::tests::dropping_receiver_releases_listener_and_stalled_handshakes` | 通过。 |
| `cargo test --locked -p geoledger-client --test client` | 2 个 Rust SDK 测试通过，覆盖远端明文限制和原发布请求恢复。 |
| `npm --prefix web test` | 80 个测试通过，0 个跳过。 |
| Web `run lint`、`run typecheck`、`run format:check`、`run build` | 全部退出 0，完成生产构建。 |
| `./scripts/test-sdks.sh` | Python、Rust、TypeScript、Go 连接实际服务完成业务流程；Python 9 个、TypeScript 14 个单元测试及 Go 测试通过。 |
| `python3 scripts/test-console.py`，使用 `--chromium /usr/bin/chromium` 和一次性服务参数 | 通过：登录/CSRF、精确 JSON、发布中断与过期登录恢复、工作区分页、历史撤销、冲突和手机布局。 |
| `python3 scripts/test-console-shadcn.py`，使用相同沙箱浏览器及测试凭据 | 通过：键盘交互、焦点恢复、成员权限弹窗、冲突按钮、删除确认及手机布局；没有页面异常。 |

总检查日志中的 Rust 测试调用累计为 153 次通过、0 次失败、30 次 ignored；该计数包含独立引擎检查与工作区检查的重复执行。Python benchmark 回归发现 19 项，执行 16 项，3 项因未设置显式 SpatiaLite 测试扩展而跳过。ignored/skip 项均单独记录，不计作通过。PostGIS、PostgreSQL TLS、百万要素容量及外部 GeoJSON 用例保留其显式启用条件。

## 依赖审计

原锁文件执行 `cargo audit --deny warnings --file "$BASELINE_LOCK"` 返回 1，复现 RUSTSEC-2026-0346、0347、0348、0349。更新后的锁文件通过 `cargo audit --deny warnings`，其 347 个锁定依赖按现有策略完成扫描。RustSec 数据库提交为 `edb58f5974b83eb8302332821cb6d389777aa351`。

| 生态 | 命令 | 结果 |
|---|---|---|
| Rust | `cargo audit --deny warnings` | 通过；四条 rustls 公告消失。 |
| npm | 对 `web` 和 `sdk/ts` 分别运行 `npm audit --omit=dev --audit-level=moderate` 及 `npm audit --audit-level=high`，使用对应 `--prefix` | 四次检查均报告 0 个漏洞。 |
| Python | `pip-audit --strict ./sdk/python` | 通过，报告无已知漏洞。 |
| Go 在线入口 | 在 `sdk/go` 中运行 `govulncheck ./...` | 失败：网络代理对 `https://vuln.go.dev/index/modules.json.gz` 返回 403。执行环境外重试结果相同。 |
| Go 官方源码快照 | 在 `sdk/go` 中运行 `govulncheck -db "file://$VULNDB_REPO/data/osv" ./...` | 通过，报告 `No vulnerabilities found.`。 |

Go 快照来自 [golang/vulndb 提交 d5e0cfac](https://github.com/golang/vulndb/commit/d5e0cfac5d260523df858cd4c0f849e248468fb6)，提交时间为 2026-10-10，执行时与该仓库公开 HEAD 一致。扫描器直接读取全部 4,613 条官方 OSV 记录，包含 1,435 个模块，未过滤公告。源码快照检查与线上数据库检查分别报告。

标准 `./scripts/audit.sh` 在 Go 在线读取处退出 1；Rust 与四次 npm 检查完成，Python 后续被单独执行。完整在线门禁仍需网络可达的 runner 验证。

## 文档核验

README 编写参考指定的 [readme-writer skill](https://github.com/Zeyad-101/Readme-writer-skill/blob/main/SKILL.md) 及其 Markdown 章节、写作和模板规范，仅进行读取。仓库原先仅有中文根 README；本次增加 [README.en.md](../README.en.md)，与 [README.md](../README.md) 对齐章节、配置、代码示例和链接目标。

能力说明以 Cargo/SDK manifests、服务启动参数、Compose/Dockerfile、Python SDK、HTTP 运维路由、存储与发布实现，以及上表中的测试为依据。文档只描述源码已有能力，实测状态和环境限制集中于本记录。

文档阶段执行结果：

| 检查 | 结果 |
|---|---|
| `python3 scripts/check-docs.py` | 通过：18 份文档的 264 个仓库内链接及当前术语检查。英文 README 已纳入检查范围。 |
| 中英文一致性核对 | 两份 README 各 295 行，13 个代码块逐字一致，标题层级一致，行内代码标识、版本、配置值和链接目标的多重集合一致；正文逐节核对。 |
| 示例语法 | 所有 shell 代码块通过 `bash -n`；Python 示例通过 AST 语法检查。 |
| Python 示例实际执行 | 从 README 原文提取代码，连接一次性 SQLite 服务；创建项目和数据集、保存、发布、按修订读回全部成功。额外断言确认修订为 1、要素 ID 正确、整数 `9007199254740993` 和坐标 `[120.15, 30.27]` 精确保留。 |
| `python3 scripts/check-env.py` | 通过：43 个模板变量与 35 个参数核对。 |
| `python3 -m py_compile scripts/check-docs.py`、`git diff --check` | 通过。 |
| 既有修复补丁反向检查 | `git apply --reverse --check` 通过；三处 CI 修复内容完整保留，未撤回其变更。 |
| README 显式外部超链接 | 两份 README 共用的 GitHub Issues 链接返回 HTTP 200。 |
| `python3 scripts/check-docs.py --external` | 退出 1：既有 `docs/api.md` 的说明性地址 `https://host:port` 被检查器当作真实 URL，抛出 `InvalidURL: nonnumeric port`；全仓外链探测因此没有完成。 |

本阶段只改动文档及文档文件枚举规则，未重复执行整套 Rust/Web 测试；代码与浏览器检查的实际结果以此前执行记录为准。文档示例新增的实际运行单独列于上表。旧 README 中预设容器内私有 Key 文件的 CLI 调用改为明确的初始化、登录、创建个人 Key 和本地 CLI 流程。

## 待补验证与交付边界

- Windows 原生 Clippy、工作区测试和 SpatiaLite 启动检查需 Windows runner 执行；Linux 检查不代替这些结果。
- Ubuntu 24.04 的系统 Chrome / AppArmor 组合需实际 GitHub runner 验证。本地沙箱浏览器成功的环境为 Debian Chromium。
- 默认在线 Go 审计受上述 403 阻断；官方源码快照的成功结果不表示线上端点已恢复。
- PostGIS、容量及其他显式可选测试本轮保持未执行状态。
- 本轮没有部署、发布或推送，也没有触发新的远端 CI。

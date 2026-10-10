# 构建环境与服务镜像修复记录

[项目概览](../README.md) · [供应链修复](supply-chain-fixes.md) · [生产运行](production.md)

## 范围与当前状态

本轮在前序 R-001 至 R-021 修复基础上，处理构建镜像告警、补齐实际服务端镜像验证，并将构建阶段纳入发布门禁。业务代码和存储格式 12 保持不变。变更保留在本地工作区，没有自动提交、推送、发布或部署到业务数据库。

本地镜像是当前工作区的验证候选，revision 标签引用评审基线，不构成已发布提交证明。正式流水线仍要求所有质量检查、构建和发布使用同一准确提交 SHA。

## Node 与 npm 构建环境

[Web Dockerfile](../web/Dockerfile) 的工具阶段使用固定 digest 的 distroless Node 22 debug 镜像，增加的 BusyBox 仅用于构建命令。运行阶段继续使用非 root 的精简 Node 镜像，只有构建输出进入运行阶段。

npm 12.2.0 及其内置依赖的刷新版本、来源、SHA256 和 registry integrity 记录在 [Node 构建锁文件](../.security/node-build.lock.json)。刷新 brace-expansion 5.0.12、http-cache-semantics 4.3.0、undici 6.28.1 后，才首次执行 npm。[校验脚本](../scripts/verify-node-build.cjs) 同时检查 Node engine、实际包版本和引用这些依赖的版本范围。

缓存库公告存在上游适用性争议。本轮记录的是旧版本扫描命中、新版本不再匹配及构建回归通过，不将其表述为已证明存在、并已复现修复的跨用户凭据泄露。

## 保留 TypeScript 版本，重建原生编译器

完整构建镜像扫描识别出 TypeScript 7.0.2 预编译文件携带旧 Go 标准库和 x/text。项目的业务 Go SDK 已升级，不会自动改变 npm 包内的独立可执行文件。

本轮使用原 npm 发布对应的 TypeScript 源提交 `2bd066d87f5bafd315be9f40889d0a60b9e58e0b`，保留编译器版本 7.0.2，以 Go 1.27.2 和锁定的更新依赖重新构建。没有降级 TypeScript，也没有为消除扫描记录而删除实际使用的编译器。

[重建脚本](../scripts/rebuild-typescript.cjs) 使用 [源与工具链锁文件](../.security/typescript-build.lock.json)、[go.mod](../.security/typescript-go.mod) 和 [go.sum](../.security/typescript-go.sum)。下载大小和校验和通过后才解包；Go 构建使用只读模块解析。安装 npm 依赖时禁止执行安装脚本，并在 SDK 与 Web 编译之前替换精确匹配版本的原生编译器。

源代码、Go 工具链、编译器和开发依赖均保留在独立扫描的构建阶段；运行镜像只复制应用输出。该流程管理容器构建，不会改写开发者机器上的全局 Node、Go 或 TypeScript 安装。

## 服务端容器运行环境

[服务端 Dockerfile](../Dockerfile) 改用固定 digest 的 Rust 1.98 / Alpine 3.24 构建环境及 Alpine 3.24 运行环境，继续使用 UID 10001、独立数据卷和内置 probe。运行环境包含 SpatiaLite、OpenSSL 及其实际依赖；扩展路径显式为 `/usr/lib/mod_spatialite.so.8`。

构建启用动态 CRT，使 SQLite 可以加载原生扩展。该变化限于容器构建，不改变独立 Windows 或 GNU/Linux 二进制的目标配置。

最初构建扫描中的 34 项匹配均来自 Cargo 下载缓存内上游项目自己的开发锁文件，而非 GeoLedger 的解析结果。最终构建保留仓库根锁文件、实际 compiler-artifact JSON 和 rustc 信息，清理下载缓存；没有修改上游缓存锁文件的版本字段，也没有增加全局漏洞豁免。语言依赖门禁继续单独审计完整仓库锁文件。

## 构建阶段与运行阶段必须同时过门禁

[镜像工作流](../.github/workflows/secure-images.yml) 先产生完整 build 阶段的双架构 OCI 布局，按 server-build 或 console-build 策略扫描，再用通过扫描的准确 OCI digest 作为命名构建上下文。最终运行阶段不会再次执行 npm、Cargo 或下载开发依赖。

随后扫描运行镜像，并保留原有全镜像完成屏障、源 SHA、配置 digest、架构、报告哈希和有效期检查。构建环境的通过凭据不能作为应用镜像发布凭据。扫描失败、缺少清单或构建阶段被跳过都会阻止发布。

本机 Docker 内嵌构建器不能读取此 OCI 上下文。工作流已固定实际验证通过的 docker-container BuildKit 镜像，并由 [策略检查](../scripts/check-release-policy.py) 防止退回不匹配的驱动。

## 本轮实际验证

以下镜像均为实际构建的 linux/arm64 候选，扫描器为锁定的 Trivy 0.75.0，HIGH / CRITICAL 阻断数均为 0：

| 对象 | 保留的扫描内容 |
|---|---|
| Web 完整构建阶段 | 14 个系统包、452 个 Node 包记录、Go 工具链和原生编译器 |
| Web 运行阶段 | 14 个系统包和 36 个 Node 包记录 |
| 服务端构建阶段 | 174 个系统包、仓库根 Cargo 依赖清单；另保留实际构建证据 |
| 服务端运行阶段 | 39 个系统包和 343 个 Cargo 包记录 |

包记录数不是唯一漏洞数；0 仅表示本次镜像在本次公告库下没有对应严重程度的匹配，不代表未知漏洞或其他架构也为 0。

完整 `scripts/check.sh` 在配置独立 PostgreSQL / PostGIS 后退出码为 0，覆盖 SQLite 与 PostGIS 全量一致性套件、连接与锁回归、热空间执行计划、原表事务、存储、双向导入导出、多实例及备份恢复。临时 PostgreSQL 实例已经正常关闭。

Alpine release 环境另行通过 26 项原生存储一致性测试；运行容器在关闭网络的条件下完成两轮初始化、就绪探针及正常退出重启。SDK 测试、Web 59 项测试、类型检查、格式检查和生产构建均通过；重建编译器的 Trivy 与 govulncheck 检查通过，四语言在线依赖审计通过。

本轮供应链策略和失败注入用例共 49 项，通过 actionlint。实际运行镜像的 1,604 个应用输出文件与已扫描构建阶段逐项哈希一致；服务端及 CLI 二进制也与通过扫描的构建阶段保持相同 SHA256。

## 验收边界与维护

Windows 实机、linux/amd64 实际镜像，以及 GitHub 上完整质量与发布工作流仍需各自执行结果。本轮认证 API 与浏览器探针被执行环境安全检查拦截，没有执行，不计为通过，也不拿前一轮旧镜像的浏览器结果替代。长时间网络分区、COMMIT 边界故障、原生 sanitizer 和生产容量仍按专项验收。

基础镜像更新由 Dependabot 提出。npm 内置依赖及 TypeScript 重建锁需随上游版本有计划地刷新：同时更新源摘要、模块文件摘要和对应验证。包版本与重建版本漂移会使检查失败，不能静默回退到旧预编译文件。上游正式包采用已修补构建后，再评估移除重建步骤；本轮不为减少步骤而放宽门禁。

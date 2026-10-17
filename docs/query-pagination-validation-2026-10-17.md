# 注册命名与查询分页整合验证（2026-10-17）

本记录验证 #36、#37、#38 合并后的同一份源码，基线为 `d1022007be7fef1963412ed490a9d653ff8a528e`，分支为 `integration/issues-36-38`。整合提交依次为 `e4105fc`、`50b8073`、`d3215b5`，并包含 `7a51e85` 中的两处回归调用适配。下述本机结果针对该整合提交，后续 CI 配置修复见末节。独立任务分支先前的验证不作为本记录的通过依据。

## 行为与兼容性

- #36：四语言 SDK 与 Web 的表注册接口统一为 `register_table`／`RegisterTable`／`registerTable`，不保留旧 `track` 别名。数据库外部写入跟踪仍使用原 tracking 术语；底层 `CreateDataset` 协议不变。
- #37：features 仅在同一读取快照、过滤条件及工作区覆盖下确有后续结果时返回游标。SQL 后端仅探测键，不读取下一条属性或几何；32 条读取批次及当前页预算保持不变。
- #38：diff、commit、history、audit 采用同事务中的仅键后续探测，整页终页无需追加请求空页。history 按修订号升序；history／audit 游标仅描述本次快照，后续新增内容仍可从最后观察到的记录继续查询。
- HTTP 与四语言 SDK 的 history 返回包含 `commits` 的页对象，调用方须适配。Protobuf 新增可选字段保留原字段编号；Web 使用页级 continuation 判断结束。
- 自定义 `RepositoryTransaction` 后端须实现新增的四个页级探测方法；features 提供 limit=1 的默认实现，后端可覆盖以避免读取负载。存储格式仍为 16，无数据库迁移。audit 仍仅允许项目 owner 读取。

## 本机验证

原生 macOS arm64，Rust 1.92.0、Node.js 22.23.1、Go 1.27.2；SDK、生成与浏览器使用 Python 3.13 venv。SQLite 使用可信 Homebrew SpatiaLite；PostGIS 使用本任务独立创建的 PostgreSQL 17.10／PostGIS 3.6 临时集群。服务使用专属回环端口和一次性凭据，Chrome sandbox 保持启用。

| 检查与命令 | 整合后的实际结果 |
| --- | --- |
| 设置独立 `GL_TEST_DATABASE_URL` 后，`RUST_TEST_THREADS=1 ./scripts/check.sh` | 通过：布局、文档、ast-grep、Python、fmt、Clippy、独立引擎、工作区、备份恢复和全部脚本内 PostGIS 可选步骤 |
| `cargo test --locked --workspace`（上述脚本内） | 142 项通过，默认 24 项标为 ignored；其中 PostGIS 项由后续可选步骤另行运行 |
| SQLite／PostGIS conformance | 各 35 项通过；覆盖精确终页、跨 32 条批次、空间过滤与草稿覆盖、无重复遗漏、响应预算及快照 |
| 脚本内 PostGIS 可选步骤 | 共 55 项通过：review 7、热查询 5、审计快照 1、conformance 35、认证 2、已有表 2、存储／导入导出／多实例各 1 |
| `cargo build --locked --workspace --bins --examples` | 通过 |
| `./scripts/test-sdks.sh`，分别连接 SQLite／PostGIS 服务 | 两种后端均通过：Go fmt／vet／race、Python 11 项、Rust 实际示例、TS 17 项，以及四语言创建／编辑／发布重试／读取 |
| TypeScript SDK build／lint／test | 通过，17 项测试 |
| Web test／typecheck／lint／format:check／build | 全部通过，90 项测试，生产构建成功 |
| `scripts/test-console-auth-http.py`，两种后端 | 均通过；包含真实 history 页对象读取、精确 GeoJSON、会话／API Key／权限与撤销流程 |
| `scripts/test-console-pagination.py`，两种后端 | 均通过：空／20 条 history、50 条 diff／commit、80 条 audit 无重复遗漏，以及桌面／手机终页控件 |
| `scripts/test-console-postgis.py` | 真实 Chrome 中完成表注册、推断模型、初始快照、编辑及发布；通过 |
| SDK 协议重新生成 | 固定 grpcio-tools 1.78.0、Go 插件 1.36.12／1.5.1、ts-proto 2.13.0；6 个生成绑定与 2 个空 init 文件摘要均不变 |
| 最终布局／文档／fmt／`git diff --check` | 通过 |

首次完整运行发现 #36 的 PostGIS 回归仍按数组读取 history，已改为 `history["commits"][0]`，保留 `Register table` 断言。调用方复查还修正了认证 HTTP 回归的同类读取；两项修正均在本次最终验证源码中。

另一次默认并行运行触发未修改的认证计时测试 `bootstrap_and_login_share_one_deadline_and_roll_back` 失败；随后明确使用 `RUST_TEST_THREADS=1` 的完整脚本通过。这里不声称默认并行模式全部通过，也未放宽或删除该断言。初次失败、定向复测、串行全量结果均保留在任务外部证据中。

容量、原生大文件和需额外证书环境的 Postgres TLS 可选套件未运行；本机验证阶段不包含 Linux／原生 Windows／远端 CI。测试 Web、后端及临时 PostgreSQL 均已停止，一次性数据库与私有凭据已清理。 上述本机验证完成时仅有本地提交，未关闭 issue、发布或部署；后续推送与 CI 修复另行记录。

## 源码对应关系

测试在仓库外的逐文件副本执行；运行前校验 415 个受跟踪文件与集成工作区一致。以下摘要排除本报告自身，覆盖其余 414 个文件，并在 `7a51e85` 提交前再次核对；后续 CI 配置修复不在此本机摘要范围内。计算方式：按路径排序，对每个文件拼接 UTF-8 路径、NUL、文件 SHA-256（小写十六进制）、LF，再对整体计算 SHA-256。

`4072203715d8083108a1822c8cf6946ec0c16984eebcb2f0e2e4c8c8c18c9870`

报告更新后另行执行文档与布局检查。验证日志、数据、凭据、截图和缓存均不进入仓库。

## 推送后的 CI 配置修复

[`7a51e85` 的常规 CI](https://github.com/mapseekai/geoledger/actions/runs/38994688878) 中，PostGIS 浏览器任务在 Ubuntu 启动阶段失败：统一使用沙箱后，原先选择的 Playwright 下载版 Chromium 不受系统现有 AppArmor 配置支持，失败发生在浏览器启动阶段，未执行页面验收。

PostGIS 任务现与已通过的 Web 任务一致，使用 `/opt/google/chrome/chrome`，仅安装所需浏览器系统依赖并检查可执行文件。Chrome sandbox 保持启用，未修改系统安全限制。该修复只涉及 CI 浏览器选择与此记录；最终推送提交仍须通过自己的整套常规 CI，不能用先前本机结果替代。

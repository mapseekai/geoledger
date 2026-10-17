# 查询页级游标验证记录（2026-10-17）

本记录针对 [#38](https://github.com/mapseekai/geoledger/issues/38)，基线为 `d1022007be7fef1963412ed490a9d653ff8a528e`，独立分支 `fix/issue-38-query-pagination`。仅覆盖 diff、commit、history、audit；features 由 #37 单独处理。

## 行为与兼容性

- 终页的 `next_after` 为 null／可选字段缺席，只有同一读取快照内存在后续记录时才返回游标。整页终页无需再请求空页确认。
- 通过后端事务中的索引键存在性探测确认后续；不读取未返回的属性、几何或审计正文。原有 diff/commit 的 32 条数据库批次及响应预算保持不变。
- history 按修订号升序；history/audit 的终页仅描述本次快照。之后新增记录可用最后观察到的修订号／事件 ID 重新查询，各次请求不共享冻结快照。
- HTTP history 与四语言 SDK history 改为包含 commits 的页对象。Protobuf 新字段保留已有字段编号；Rust/Go/TS 的 HistoryPage 与 Python 字典、Web 页对象同步使用页级游标。
- RepositoryTransaction 新增四项仅键后续探测语义，自定义后端须同步实现。存储格式仍为 16，无数据库迁移。audit 继续仅对项目 owner 开放，平台管理员没有读审计旁路。

## 环境

原生 macOS 15.6.1（24G90）、arm64；Rust 1.92.0；Node.js 22.23.1、npm 10.9.8；Go 模块工具链 1.27.2；Python 检查为 3.14.5，SDK／生成／浏览器使用独立 Python 3.13.13 venv。

SQLite 使用 Homebrew SpatiaLite 5.1.0。PostGIS 在本次任务自行创建的临时 PostgreSQL 17.10 集群、数据库 `geoledger_test` 上验证，PostGIS 版本为 3.6。SDK 服务与浏览器均使用专属回环端口、一次性数据和私有测试凭据；未操作现有服务或其他任务数据库。

所有 Cargo 验证通过 `CARGO_TARGET_DIR` 使用独立临时输出目录；SQLite 验证设置 `GL_SPATIALITE_EXTENSION` 为已安装的可信 Homebrew 扩展路径。

## 实际结果

| 检查 | 最终结果 |
| --- | --- |
| 基线新增回归 | 复现失败：diff 缺失页级字段，history 返回数组 |
| `./scripts/check.sh` | 通过：布局、文档、ast-grep、Python 脚本、fmt、Clippy、独立引擎与工作区测试、备份恢复演练 |
| SQLite conformance | 31 项通过；包含空／短终／整终／中间页、跨 32 条批次、无重复遗漏、后续新增和大下一条不消耗当前页预算 |
| 真实服务 service 套件 | 3 项通过；新增 HTTP/gRPC 页元数据一致及平台管理员读取他人项目 audit 被拒绝 |
| PostGIS conformance | 31 项通过：`GL_CONFORMANCE_BACKEND=postgis cargo test --locked -p geoledger-engine --test conformance -- --test-threads=1` |
| PostGIS 审计快照 | `cargo test --locked -p geoledger-engine --lib postgis_audit_cursor -- --ignored --test-threads=1` 通过，页读与探测之间发生提交仍保持同一快照 |
| `cargo build --locked --workspace --bins --examples` | 通过 |
| `./scripts/test-sdks.sh` | SQLite、PostGIS 专属服务均通过：Go vet／race、Python 10 项、Rust 实际示例、TS 16 项及实际四语言创建、编辑、发布重试和读取 |
| `npm --prefix sdk/ts run lint` | 通过 |
| Web test／lint／typecheck／format:check／build | 全部通过；Web 测试 88 项，Node.js 22 下生产构建成功 |
| `scripts/test-console-pagination.py` | 真实 Chrome、开启 sandbox，通过空历史、20 条整页历史、50 条 diff/commit 整页终页、80 条 audit 无重复遗漏、桌面／手机结束控件 |
| SDK 生成复现 | 固定 grpcio-tools 1.78.0、protoc-gen-go 1.36.12、protoc-gen-go-grpc 1.5.1、ts-proto 2.13.0；再生成后 7 个绑定文件摘要不变 |
| `git diff --check` | 通过 |

完整 check 的 `GL_TEST_DATABASE_URL` 未设置，因此它自己的可选 PostGIS 分支明确跳过；上述 PostGIS conformance、审计快照和 SDK 联调为额外独立执行结果，不表示全部 opt-in PostGIS／容量／负载套件都已执行。未运行 Linux 或原生 Windows 检查，也未发布、部署或推送。

本地日志位于 worktree 的 `target/issue38-check.log`、`target/issue38-postgis-conformance.log`、`target/issue38-sdks-sqlite.log`、`target/issue38-sdks-postgis.log`、`target/issue38-browser.log`；浏览器截图位于 `target/issue38-browser/`。它们是未提交的验证产物。临时测试服务在验收后停止。此记录说明本机验证范围，不作部署性能或容量承诺。

## 整合注意

与 #37 预计共享 query.rs、conformance、proto 生成绑定、SDK 模型／转换及 API 文档等文件；保留两个任务各自的分页实现和回归，协议合并后按固定工具重新生成绑定。父任务需在最终整合提交上重新运行统一验证，本分支的结果不替代整合后验证。

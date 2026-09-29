# 验证记录：2026-09-21

> 本文保留改名前的历史命令、路径与实测结果；当前项目名为 `geoledger`，命令为 `gl`，配置使用 `GL_*`。重命名与兼容边界见 [重命名说明](rename-to-geoledger.md)。

## 实测环境

macOS Apple Silicon；Rust 1.92.0；protoc 35.1；隔离容器中的 PostgreSQL 17.7 / PostGIS 3.6.1。

首次验证完成于目录迁移前。同一套实现现已提升至项目根目录，旧骨架已移出活动工作区。下列结果为首次验证记录，迁移后的复验单独记录。

## 检查结果

| 检查 | 结果 |
|---|---|
| cargo fmt --all -- --check | 通过 |
| cargo clippy --workspace --all-targets -- -D warnings | 通过 |
| cargo test --workspace | 20 项通过；6 项 PostGIS 测试明确 ignored |
| 显式运行 PostGIS 集成测试 | 6 项通过 |
| cargo build --workspace --bins --examples | 通过 |
| CLI --version | spatial-version 0.1.0 |

合计 **26 项 Rust 测试实际通过**。默认 ignored 的测试已另行显式执行，不将“未执行”计为通过。

## PostGIS 集成验证

1. 导入、精确 numeric 文本、带 Z 的 XDR EWKB、无效更新不产生 diff、主键修改跟踪。
2. 不同属性三方合并、双父提交、同字段冲突、选择 theirs、继续合并。
3. 未提交修改保护、restore 显式确认、revert 新建历史、reset 恢复旧提交。
4. 两侧几何变化产生冲突，取消后工作副本与 HEAD 保持原状。
5. 合并结果违反唯一约束时，数据库与 HEAD 均不改变，仓库 fsck 通过。
6. 增加字段导致 schema 指纹变化时拒绝普通数据提交。

测试代码：`crates/app/tests/postgis.rs`。它拒绝数据库名不是 spatial_version_test 的连接，使用随机 schema，测试数据保留在隔离容器中。其他测试覆盖持久化对象校验、锁释放、三方规则、确定性树和一万记录下的路径复用。

## 四入口端到端检查

另用临时仓库 `/tmp/spatial-version-cli-smoke-_eaq4b0s` 和随机测试 schema 做实际调用，不只检查 mock：

- CLI 初始化、导入 PostGIS 表、检查修改、提交、创建分支、fsck：通过。
- 启动实际 HTTP / gRPC 服务后，HTTP 读取状态、通过 HTTP 提交数据库编辑：通过。
- HTTP 无令牌返回 401；真实 gRPC 客户端无令牌返回 Unauthenticated：通过。
- gRPC Rust 客户端读取真实 PostGIS 工作副本：通过。
- Rust 库示例读取同一仓库：通过。
- 四种入口读到同一个 HEAD：通过。

端到端检查以临时 Python 驱动执行；可运行客户端示例位于 `crates/app/examples/library.rs` 和 `crates/server/examples/client.rs`。临时 HTTP/gRPC 服务已正常停止。没有新增后台定时任务或外部云资源。

## 重现基础测试

```bash
cargo build --workspace --bins --examples
./scripts/check.sh
# 另行准备隔离 PostGIS 测试库，并显式设置连接：
export SV_TEST_DATABASE_URL='postgresql://USER:PASSWORD@127.0.0.1:PORT/spatial_version_test'
cargo test -p spatial-version --test postgis -- --ignored --test-threads=1
```

未运行数据库测试时，check.sh 会明确说明；不能把默认 cargo test 当成完成数据库兼容验证。

## 尚未验证

没有百万/千万要素性能基准；没有 Windows/Linux 执行验证；没有依赖安全审计；没有断电、kill -9、磁盘满、网络中断、数据库 failover 的恢复故障注入；没有多租户或高并发业务数据库验证。恢复日志协议已实现，但正常回滚测试不能证明所有崩溃窗口均正确。因此本版本仍是开发验证用 MVP。

## 提升至项目根目录后的复验：2026-09-21

当前唯一 Cargo workspace 根目录为 `/Users/zhang/code/spatial-version`。
六个 crate、Cargo.lock、示例、测试和脚本均已移动至根目录；业务源码与移动前的内容哈希一致。
原根目录骨架已移至 `.reference/legacy-scaffold-20260921-132236/`，该备份和 Kart 参考资料均由 Git 忽略，不参与 Cargo 构建。
原 Git 元数据保持不变；README 与开发说明已更新为根目录入口。

| 根目录复验 | 结果 |
|---|---|
| cargo metadata --no-deps --format-version 1 | 根目录正确，识别六个 workspace crate |
| cargo fmt --all -- --check | 通过 |
| cargo clippy --workspace --all-targets -- -D warnings | 通过 |
| cargo test --workspace | 20 项通过，0 项失败，6 项 PostGIS 测试 ignored |
| cargo build --workspace --bins --examples | 通过 |
| ./target/debug/spatial-version --version | spatial-version 0.1.0 |
| 活动文件中的旧嵌套路径 | 无残留 |

本次目录迁移未重新启动 PostGIS 测试容器，也未重新执行六项数据库集成测试；首次数据库与四入口端到端验证结果仍单独记录在上文，不计入本次复验通过数量。
迁移复验日志保存在 `.reference/root-verification.log`。本次未提交或推送 Git。

## 真实大数据与超时配置复验：2026-09-22

新增可配置的事务级 PostGIS 语句超时，默认仍为 120 秒。本地真实数据库上 28 项测试（含显式启用的集成测试）全部通过，格式与 Clippy 检查通过。

用户提供的 lucc.geojson（9,384 条）完成版本操作与全表校验；big.geojson（483,268 条、23,040,506 个顶点）首次触发 120 秒超时，回滚一致性检查通过。配置 900 秒后完整初始版本导入用时 123.575 秒，10,000 条增量提交用时 119.086 秒，切换、合并、revert、全表校验均通过，fsck 校验 1,207,551 个对象用时 498.086 秒。

完整规模、计时、内存测量范围、日志位置与未测边界见 [大数据验证记录](big-data-verification.md)。此结果不代表全量修改或并发性能保证。

## 字段结构与性能优化回归：2026-09-22

完整工作区测试显式启用专用 PostGIS 测试后 **40 项通过、0 项失败、0 项忽略**，其中 16 项是 Application → 真实 PostGIS 集成测试。格式检查、Clippy（warnings as errors）通过。日志保留在 `.reference/schema-final-tests.log` 和 `.reference/schema-final-clippy.log`。

新增验证覆盖：四种工具字段命令；外部 DDL 检测/提交/丢弃；稳定 ID 改名和同名删除后重建；删列值恢复；numeric → integer 后精确小数恢复；类型转换/注入输入/主键/索引依赖拒绝及回滚；保留字列名与 UNIQUE；结构改名结合另一分支数据编辑；独立新增字段合并；不兼容改名和删列/数据编辑冲突拒绝；数据库已提交、本地状态未发布的 journal 恢复；v1 数据集升级；超过单批的插入/更新/删除；历史 diff 截断与结构差异；对象压缩复用下空对象和损坏载荷校验。

上文首次验证中的“新增字段导致漂移时拒绝”是旧版行为；现在 v2 支持受控字段演进，测试改为拒绝未支持的可空性变化。新命令共用 HTTP/gRPC 的 Command 路由，但本轮字段操作未另做网络传输端到端测试。真实大表数据与字段计时继续记录在 [大数据验证记录](big-data-verification.md)。

整表对象批写另验证临时对象读取、对象类型校验、重复对象去重、扫描失败后可重试、外层事务回滚及重新打开仓库后的持久性。临时表和排序写入不改变不可变对象编码，也不放宽 synchronous=FULL。

另用真实旧二进制创建 v1 小仓库，再运行新二进制 upgrade、字段改名并 reset 到原始提交：原值恢复、status 干净、fsck 通过；旧二进制打开升级后的仓库明确返回不支持的仓库格式。记录位于 `.reference/schema_legacy_9d78384d/results.json`。

48 万条 big 数据上的四种字段命令、删列值历史恢复、外部新增字段/status/commit、历史 diff 限量输出均通过。最终全表摘要匹配初始数据，工作副本干净；fsck 校验 4,109,530 个对象并成功。续测记录为 `.reference/large-data-20260922/big_optimized_cacb7424/schema_optimized/results.json`（verified=true）。结构回填、完整结构差异遍历和历史库增长的成本已在大数据记录中单独披露。


## Rust 审查修复回归（2026-09-22）

当前完整工作区显式启用 PostGIS 测试后 **50 项通过、0 失败、0 忽略**，包含 21 项 Application → PostGIS 测试。fmt、Clippy warnings-as-errors、Rust 1.88 全目标编译检查通过。四个原始错误复现已重跑，并增加分页字节预算/恢复、合并基点读取规模、树根变化差异和错误来源/编码边界回归。

同一组 10,000 条脏记录、每条 8 KiB 文本的 debug `status --limit 1` 实测，峰值 RSS 从 184.30 MiB 降到 33.56 MiB，耗时基本持平。详细行为、自动合并的保守转换策略、查询索引补建以及测试边界见 [Rust 审查修复记录](rust-review-fixes.md)。证据目录为 `.reference/reviews/rust-fixes-20260922/`。


## Volo Thrift 接入验证（2026-09-22）

- `cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets -- -D warnings` 通过。
- `cargo +1.88.0 check --workspace --all-targets --offline` 通过；该工具链补装 rustfmt，Volo 构建器需要它生成代码。
- 指向 `spatial_version_test` 的 `cargo test --workspace --offline -- --include-ignored`：53 项通过，0 失败、0 忽略。
- 新增真实 TCP Thrift 测试：认证拒绝、init/branch/status/log、与同步 Application 状态一致、负 limit、非法 JSON、缺失引用、超过 4 MiB 的 request，以及连接排空。
- 新增真实 PostGIS Thrift 测试：通过 execute 导入测试表，新增字段、改名、修改类型、删除字段；验证规范类型、最终表结构与原记录、6 条提交历史、clean 状态及 fsck。测试拒绝任何名称不为 `spatial_version_test` 的数据库。
- 独立临时仓库中运行 CLI `serve-thrift --thrift 127.0.0.1:0`，示例客户端携带 token 查询成功；SIGTERM 后进程以 0 退出。

本轮验证 Volo Rust 客户端和 Framed Binary 协议；没有测量 Thrift 与 gRPC 的吞吐差异，也未运行其他语言客户端兼容性测试。自动生成代码的 unsafe 边界见 [API 文档](api.md#生成代码与内存安全边界)。

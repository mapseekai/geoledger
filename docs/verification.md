# 验证记录：2026-09-21

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

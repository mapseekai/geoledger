# Kart 源码参考记录

参考来源：`koordinates/kart`。本次实际读取的是提交 **84d01ec62ff5503065f0963202c6fd3f971c334b**（2026-08-24），不是笼统引用某个可能变动的 latest。

| 实际查看的源码 | 参考点 | 本实现 |
|---|---|---|
| kart/tabular/v3.py: TableV3、encode_feature、encode_pks_to_path | 数据集元数据与要素独立，schema / legend 与 PK 路径映射 | schema + record 独立对象；v2 schema 已有稳定 column ID，尚无 legend 记录布局 |
| kart/schema.py: align_to_self、diff_types（505–617 行） | 按稳定列 ID 对齐，区分新增/删除/改名/类型差异 | 稳定 ID + PostGIS attnum 映射；三方结构对齐与历史数据投影 |
| kart/tabular/working_copy/postgis.py（283–348 行） | 支持的删除、改名、类型变化原地 ALTER，其他情况重写工作副本 | 工具操作事务内 ALTER；跨结构历史恢复从对象完整回填，避免反向 cast 丢失原值 |
| kart/tabular/working_copy/postgis.py: create_common_functions | INSERT / UPDATE / DELETE 跟踪 OLD 与 NEW PK；冲突忽略去重 | 自有 dirty set 和触发器，另加 TRUNCATE 拦截和绑定验证 |
| kart/merge.py: do_merge、move_repo_to_merging_state | merge-base、快进、三方树合并、独立的冲突状态 | Rust 原生 DAG、字段级合并、候选树与继续/取消状态机 |
| kart/merge_util.py（定义及调用关系） | ancestor / ours / theirs、持久化冲突与 resolved 索引 | 版本化 MergeState / Conflict 数据结构 |

固定源码入口：

- https://github.com/koordinates/kart/tree/84d01ec62ff5503065f0963202c6fd3f971c334b
- https://github.com/koordinates/kart/blob/84d01ec62ff5503065f0963202c6fd3f971c334b/kart/tabular/v3.py
- https://github.com/koordinates/kart/blob/84d01ec62ff5503065f0963202c6fd3f971c334b/kart/tabular/working_copy/postgis.py
- https://github.com/koordinates/kart/blob/84d01ec62ff5503065f0963202c6fd3f971c334b/kart/merge.py
- https://github.com/koordinates/kart/blob/84d01ec62ff5503065f0963202c6fd3f971c334b/kart/schema.py

## 其他空间版本管理实现：QGIS Versioning

本轮另读取 Oslandia/qgis-versioning 提交 **f6d5aa61f778bfa05c9470837845fa8f5f631b5f**。该 GitHub 仓库已归档并提示迁往 GitLab，本文记录的是固定历史源码，不称为最新实现。

实际参考 [historize.sql](https://github.com/Oslandia/qgis-versioning/blob/f6d5aa61f778bfa05c9470837845fa8f5f631b5f/versioningDB/sql/historize.sql) 对约束信息的目录化，以及 [versioning.py](https://github.com/Oslandia/qgis-versioning/blob/f6d5aa61f778bfa05c9470837845fa8f5f631b5f/versioningDB/versioning.py#L333-L400) 中归档 schema、完整行与列的处理。其数据库内历史方案和本项目的外部不可变对象库不同。这里借鉴的是“结构和依赖定义也必须纳入可恢复状态”，没有照搬其 SQL 历史布局或复制 GPL 源码。

这次实现保留原 record/v1 的字段名编码，代价是新增/改名会重编码全表并增长对象库。Kart legend 布局能分离部分结构变化与记录内容；后续若采用此方向，应另设记录格式版本与迁移，而不能悄悄改变现有对象编码。

## 刻意没有照搬的部分

Kart 借助 Git / pygit2 存储对象并进行树合并。这里独立实现版本对象、确定性 treap 和数据库恢复协议，使用 SQLite WAL + BLAKE3 + Zstd。不是 Kart 的 Rust 语言移植，不读取其 Git 数据布局，不复用其 Git 网络协议，也不复制 Python 实现。

Kart 的 schema legend、跨数据格式导入、空间过滤、栅格/点云、远程同步和复杂工作副本重置逻辑都没有被完整移植。本文记录核心源码参考，不代表对整个 Kart 仓库逐行审计。

## 许可证

参考仓库 COPYING 写明 GPL v2 与 linking exception；本项目没有复制 Kart 源文件或直接逐行翻译其代码。新增实现使用自身 MIT LICENSE。后续若实际引入 Kart 代码、依赖或资源，必须重新检查相应许可证义务，不能用“参考实现”替代许可审查。

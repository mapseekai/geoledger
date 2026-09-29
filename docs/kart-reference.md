# 空间版本管理源码参考

本页以固定提交的公开源码记录设计参考点，GeoLedger 的具体实现以仓库中的 Rust 源码为准。以下文件链接于 2026-09-29 核验可访问。

## Kart

参考提交为 `84d01ec62ff5503065f0963202c6fd3f971c334b`。

| 固定源码 | 参考主题 | GeoLedger 实现 |
|---|---|---|
| [tabular/v3.py](https://github.com/koordinates/kart/blob/84d01ec62ff5503065f0963202c6fd3f971c334b/kart/tabular/v3.py) | 数据集元数据与要素编码 | schema、record 与数据树独立对象 |
| [schema.py](https://github.com/koordinates/kart/blob/84d01ec62ff5503065f0963202c6fd3f971c334b/kart/schema.py) | 稳定列身份和结构差异 | 稳定字段 ID、三方结构对齐与历史数据投影 |
| [working_copy/postgis.py](https://github.com/koordinates/kart/blob/84d01ec62ff5503065f0963202c6fd3f971c334b/kart/tabular/working_copy/postgis.py) | 表结构演进及 OLD / NEW 主键跟踪 | 事务内字段操作、dirty set、跟踪触发器和绑定校验 |
| [merge.py](https://github.com/koordinates/kart/blob/84d01ec62ff5503065f0963202c6fd3f971c334b/kart/merge.py) | 合并基点、三方合并与冲突状态 | Rust 提交图、字段级合并、候选树和继续/取消流程 |

GeoLedger 使用 SQLite WAL、BLAKE3、Zstd 和确定性 treap 构建本地历史，使用 journal / marker 协调 PostGIS 工作副本。字段名参与当前记录编码，结构变更采用全量扫描；列身份记录布局的演进安排见 [开发计划](roadmap.md)。

## QGIS Versioning

参考提交为 `f6d5aa61f778bfa05c9470837845fa8f5f631b5f`。[historize.sql](https://github.com/Oslandia/qgis-versioning/blob/f6d5aa61f778bfa05c9470837845fa8f5f631b5f/versioningDB/sql/historize.sql) 提供约束信息组织的参考，[versioning.py](https://github.com/Oslandia/qgis-versioning/blob/f6d5aa61f778bfa05c9470837845fa8f5f631b5f/versioningDB/versioning.py) 提供数据库内结构与历史行处理的参考。

GeoLedger 将字段结构和依赖定义纳入可恢复状态，历史记录存储在数据库外的不可变对象库中。实现路径见 [架构与一致性](architecture.md)和[字段结构版本管理](schema-evolution.md)。

## 代码与许可管理

GeoLedger 源码采用仓库 [LICENSE](../LICENSE) 中的 MIT 许可证。引入外部源码、依赖或资源时，分别记录来源和适用许可证，并进行相应审查。

# GeoLedger

GeoLedger 提供本地空间数据版本管理和中心化多人协作两种运行模式，正式空间数据均由 PostGIS 承载。

## 本地版能力

| 能力 | 内容 |
|---|---|
| 数据管理 | 注册具有单列主键的普通表，跟踪记录增删改，保存属性及多个 geometry 列 |
| 字段演进 | 新增、删除、改名、修改类型，记录 SQL / QGIS 编辑产生的结构变化 |
| 历史查询 | 查看状态、差异、提交、记录、字段结构与分支变更日志 |
| 分支合并 | 创建与切换分支，快进和三方合并，查看、解决、继续或取消冲突处理 |
| 历史恢复 | 恢复工作副本、回退分支、生成撤销提交，校验仓库与恢复中断操作 |
| 接口集成 | CLI、Rust 库、HTTP、gRPC；Unix 构建提供 Volo Thrift |

## 中心版 0.2.0-alpha.1

[geoledger-center](crates/center/Cargo.toml) 提供独立的 `gl-center` 服务，面向浏览器多用户编辑中心原生要素集合。
每个用户的草稿读取固定基础修订与自身增量；发布按属性三方合并，几何整体合并，冲突支持分页查看和分批解决。服务内置浏览器协作测试台，提供 GeoJSON 编辑、差异、发布及历史验证。
已发布数据与时态历史均存储为 PostGIS 几何和 JSON 属性，可按修订、范围和游标查询。

本地 `gl` / SQLite 版适合已有业务表、离线仓库和分支操作；中心版适合共享服务、独立逻辑草稿和统一发布，
中心版提供 owner/editor/viewer 项目权限，个人令牌在服务端映射到操作身份。两个版本分别使用 `_geoledger` 和 `_geoledger_center`，
中心版保持独立应用层及格式 1；本地编解码器和命令保持原样。
启动见[快速开始](docs/getting-started.md#中心版)，接口见[API](docs/api.md#中心版)。

## 文档导航

按“开始使用 → 日常操作 → 接口集成 → 开发维护”阅读：

| 文档 | 内容 |
|---|---|
| [快速开始](docs/getting-started.md) | 本地 CLI、中心服务启动与浏览器协作入门 |
| [功能指南](docs/user-guide.md) | 本地版本操作，以及中心草稿、发布、冲突和撤销 |
| [API](docs/api.md) | Rust、HTTP、gRPC、Thrift 和统一命令协议 |
| [开发说明](docs/development.md) | 架构、源码构建、验证和 Windows 打包 |

项目采用 [MIT 许可证](LICENSE)。

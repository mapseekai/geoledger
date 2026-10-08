# GeoLedger

GeoLedger 是空间要素的版本控制服务。安装一个 `geoledger-server`，使用 SDK、远程 `gl` CLI 或内置管理界面完成多人编辑、版本发布、历史查询和冲突解决。

- **默认 SQLite**：要素、空间索引、工作区、历史、权限与审计均保存在服务端数据目录，无需外部数据库。
- **可选 PostGIS**：连接独立 PostgreSQL/PostGIS 数据库，使用事务、行锁与 GiST 空间索引。
- **统一应用层**：属性三方合并、几何整体合并、发布幂等、owner/editor/viewer 权限。
- **四语言业务 SDK**：Go、Rust、TypeScript（Node.js）、Python 直接操作项目、数据集和工作区，内部使用 gRPC。
- **浏览器 HTTP**：内置[中文管理控制台](docs/console.md)，含独立登录页、项目 / 数据集 / 工作区管理，HTTP 和 RPC 共用应用层、身份认证及并发容量。
- **可替换存储**：后端实现 [StorageBackend / RepositoryTransaction](crates/engine/src/repository.rs)，无需修改客户端协议与合并规则。

```sh
cargo build --release --locked --bins
./target/release/geoledger-server
```

首次运行创建 `./geoledger-data/geoledger.sqlite3` 与私有 `tokens.json`。浏览器打开 `http://127.0.0.1:7881`；gRPC 地址为 `http://127.0.0.1:7882`。

```sh
./target/release/gl --token-file ./geoledger-data/tokens.json info
```

| 文档 | 内容 |
|---|---|
| [快速开始](docs/getting-started.md) | 默认 SQLite、PostGIS、SDK 与 CLI |
| [使用指南](docs/user-guide.md) | 编辑、发布、冲突、历史、撤销 |
| [API 与 SDK](docs/api.md) | 四语言业务接口、HTTP 与错误语义 |
| [存储扩展](docs/storage.md) | 事务接口、数据语义、新后端验收 |
| [生产运行](docs/production.md) | 部署、TLS、身份、限制、备份与恢复 |
| [开发验证](docs/development.md) | 构建、生成 SDK、跨后端回归 |
| [SDK 审查记录](docs/sdk-review.md) | 公开接口调整、独立审查发现、修复与回归 |
| [重构验证记录](docs/production-review.md) | 本轮实现范围、验证与容量边界 |

当前版本 `0.3.0-alpha.1`；存储格式 4，面向新建库部署。采用 [MIT 许可证](LICENSE)。

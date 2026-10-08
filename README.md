# GeoLedger

GeoLedger 是空间要素的版本控制服务。安装一个 `geoledger-server`，使用 SDK、远程 `gl` CLI 或独立管理界面完成多人编辑、版本发布、历史查询和冲突解决。

团队成员可在各自工作区编辑道路、地块等空间数据，将修改检查后发布为共享版本。属性三方合并协调并发编辑，历史快照与提交撤销帮助追踪和修正变化，成员权限与审计为协作提供访问控制和操作记录。

- **默认 SQLite**：要素、空间索引、工作区、历史、权限与审计均保存在服务端数据目录，开箱即可运行。
- **可选 PostGIS**：连接独立 PostgreSQL/PostGIS 数据库，使用事务、行锁与 GiST 空间索引。
- **统一应用层**：属性三方合并、几何整体合并、发布幂等、owner/editor/viewer 权限。
- **四语言业务 SDK**：Go、Rust、TypeScript（Node.js）、Python 直接操作项目、数据集和工作区，内部使用 gRPC。
- **独立 Web 控制台**：[Next.js 管理项目](web/README.md)，使用 React、TypeScript、Tailwind CSS v4 与 shadcn/ui。浏览器使用 HTTP，Web 服务端直接调用 TS SDK。
- **可替换存储**：后端实现 [StorageBackend / RepositoryTransaction](crates/engine/src/repository.rs)，沿用统一的客户端协议与合并规则。

```sh
cargo build --release --locked --bins
./target/release/geoledger-server
```

首次运行创建 `./geoledger-data/geoledger.sqlite3` 与私有 `tokens.json`。gRPC 地址为 `http://127.0.0.1:7882`，HTTP 监控/API 地址为 `http://127.0.0.1:7881`。管理页面按 [Web 启动说明](web/README.md) 独立运行，默认访问 `http://localhost:3000`。

```sh
./target/release/gl --token-file ./geoledger-data/tokens.json info
```

| 文档 | 内容 |
|---|---|
| [快速开始](docs/getting-started.md) | 默认 SQLite、PostGIS、SDK 与 CLI |
| [使用指南](docs/user-guide.md) | 编辑、发布、冲突、历史、撤销 |
| [API 与 SDK](docs/api.md) | 四语言业务接口、HTTP 与错误语义 |
| [存储扩展](docs/storage.md) | 事务接口、数据语义、新后端验收 |
| [生产运行](docs/production.md) | 部署、TLS、身份、容量规划、备份与恢复 |
| [开发验证](docs/development.md) | 构建、生成 SDK、跨后端回归 |
| [Web 审查记录](docs/web-review.md) | 独立控制台、审查修正与浏览器验证 |
| [SDK 审查记录](docs/sdk-review.md) | 公开接口调整、独立审查发现、修复与回归 |
| [重构验证记录](docs/production-review.md) | 服务能力、验证场景与容量测量 |

当前版本 `0.3.0-alpha.1`；存储格式 5，面向新建库部署。采用 [MIT 许可证](LICENSE)。

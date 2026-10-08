# 快速开始

[项目概览](../README.md) · [API](api.md) · [生产运行](production.md)

## 构建

准备 Rust 1.88+、protoc、C 编译器、pkg-config 与 OpenSSL 开发库；推荐 Rust 1.92。SQLite 编译进二进制，运行时无需安装 SQLite 或空间扩展。

```sh
cargo build --release --locked --bins
```

生成 `target/release/geoledger-server` 和 `target/release/gl`。Windows 对应 `.exe`。

## 默认 SQLite

```sh
./target/release/geoledger-server --data-dir ./geoledger-data
```

创建服务端目录、SQLite 数据库和初始 `admin` 令牌文件；Linux/macOS 目录权限 0700、令牌文件 0600。凭证不会打印到日志。HTTP 默认监听 `127.0.0.1:7881`，gRPC 默认监听 `127.0.0.1:7882`。

打开 `http://127.0.0.1:7881`，从私有 `geoledger-data/tokens.json` 读取自己的 token，输入管理界面后连接服务。依次创建项目、数据集和工作区，然后在高级操作中编辑 GeoJSON、保存和发布。令牌只保存在当前页面内存。

CLI 连接同一个服务：

```sh
./target/release/gl --token-file ./geoledger-data/tokens.json info
./target/release/gl --token-file ./geoledger-data/tokens.json projects
printf '%s\n' '{"name":"城市道路"}' | ./target/release/gl --token-file ./geoledger-data/tokens.json call create_project
```

远程客户端设置 `GL_ENDPOINT` 和 `GL_TOKEN`。令牌文件选项方便本机管理员使用；远程应用仅需要自己的令牌，无需服务器文件。

## PostGIS

管理员先创建独立数据库和非超级用户服务账号，并在该库执行：

```sql
CREATE EXTENSION IF NOT EXISTS postgis;
```

让服务账号拥有新库的 public schema 建表权限。通过秘密管理系统注入 `GL_DATABASE_URL`，随后启动：

```sh
./target/release/geoledger-server --storage postgis --data-dir ./geoledger-data
```

连接串采用 PostgreSQL 标准 URL，生产环境使用 `sslmode=verify-full` 及可信 CA。数据库账号仅服务端持有。启动时自动初始化空库；已有格式必须为 4。选择 PostGIS 后，连接失败会终止启动。

两种后端的 SDK、HTTP 与 CLI 调用方式一致。改变后端配置会选择另一个数据库；数据迁移需要显式导出、导入与校验。

## 多用户凭证

```sh
./target/release/geoledger-server tokens --out ./team-tokens.json alice bob
./target/release/geoledger-server --token-file ./team-tokens.json
```

新令牌文件按 create-new 创建，避免覆盖。使用项目所有者账号调用 `set_member` 授予 `owner`、`editor` 或 `viewer`。凭证定义身份，项目成员关系定义资源权限。企业身份接入见 [JWT 配置](production.md#身份与网络)。

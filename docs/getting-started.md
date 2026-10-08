# 快速开始

[项目概览](../README.md) · [API](api.md) · [生产运行](production.md)

## 构建

准备 Rust 1.88+、protoc、C 编译器、pkg-config 与 OpenSSL 开发库；推荐 Rust 1.92。SQLite 编译进二进制，可直接运行默认存储后端。

```sh
cargo build --release --locked --bins
```

生成 `target/release/geoledger-server` 和 `target/release/gl`。Windows 对应 `.exe`。

## 默认 SQLite

```sh
./target/release/geoledger-server --data-dir ./geoledger-data
```

创建服务端目录、SQLite 数据库和初始 `admin` 令牌文件；Linux/macOS 目录权限 0700、令牌文件 0600。凭证保存在私有令牌文件中。HTTP 默认监听 `127.0.0.1:7881`，gRPC 默认监听 `127.0.0.1:7882`。

浏览器管理使用独立的 [Next.js 控制台](../web/README.md)。按其说明构建 TS SDK、安装 Web 依赖、配置会话密钥并启动 Web。默认访问 `http://localhost:3000`，输入私有 `geoledger-data/tokens.json` 中自己的 token。令牌存入加密的 HttpOnly 会话 Cookie。

CLI 连接同一个服务：

```sh
./target/release/gl --token-file ./geoledger-data/tokens.json info
./target/release/gl --token-file ./geoledger-data/tokens.json projects
printf '%s\n' '{"name":"城市道路"}' | ./target/release/gl --token-file ./geoledger-data/tokens.json call create_project
```

远程客户端设置 `GL_ENDPOINT` 和 `GL_TOKEN`。令牌文件选项方便本机管理员使用；远程应用通过服务地址和自己的令牌完成访问。

## PostGIS

管理员先创建独立数据库和非超级用户服务账号，并在该库执行：

```sql
CREATE EXTENSION IF NOT EXISTS postgis;
```

让服务账号拥有新库的 public schema 建表权限。通过秘密管理系统注入 `GL_DATABASE_URL`，随后启动：

```sh
./target/release/geoledger-server --storage postgis --data-dir ./geoledger-data
```

连接串采用 PostgreSQL 标准 URL，生产环境使用 `sslmode=verify-full` 及可信 CA。数据库账号仅服务端持有。启动时自动初始化空库；已有库使用格式 5。启动时校验 PostGIS 连接和存储配置，确保服务连接到指定数据库。

两种后端的 SDK、HTTP 与 CLI 调用方式一致。改变后端配置会选择另一个数据库；数据迁移需要显式导出、导入与校验。

## 多用户凭证

```sh
./target/release/geoledger-server tokens --out ./team-tokens.json alice bob
./target/release/geoledger-server --token-file ./team-tokens.json
```

新令牌文件按 create-new 创建，保护已有凭证。使用项目所有者账号调用 `set_member` 授予 `owner`、`editor` 或 `viewer`。凭证定义身份，项目成员关系定义资源权限。企业身份接入见 [JWT 配置](production.md#身份与网络)。

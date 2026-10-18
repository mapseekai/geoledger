# 安全配置

[项目概览](../README.md) · [生产运行](production.md) · [开发验证](development.md)

本文说明传输加密、凭证生命周期、数据库 TLS 与控制台会话。服务配置支持环境变量及对应命令行参数；初始管理员密码仅通过 `GL_ADMIN_PASSWORD` 环境变量注入。

## 传输加密

`geoledger-server` 可直接在 HTTP 与 gRPC 两个监听器上提供 TLS（rustls，TLS 1.2/1.3）：

| 环境变量 / 参数 | 用途 |
|---|---|
| GL_TLS_CERT / --tls-cert | PEM 证书链，与私钥同时配置后两个监听器都启用 TLS |
| GL_TLS_KEY / --tls-key | PEM 私钥（PKCS#8、PKCS#1 或 SEC1） |
| GL_TLS_CLIENT_CA / --tls-client-ca | PEM CA；配置后要求客户端证书（mTLS） |
| GL_RELOAD_INTERVAL_SECS | TLS 证书文件的变更检查间隔，默认 10 秒 |

HTTP 监听器协商 h2 与 http/1.1，gRPC 监听器协商 h2。证书文件更新后在检查间隔内生效，`kill -HUP` 立即重新加载；新文件校验失败时继续使用当前证书并记录错误日志。TLS 握手在独立任务中完成并有 10 秒期限。

监听非回环地址且未启用 TLS 时，服务启动会输出警告：此时请在可信网关终结 TLS。仓库提供两份网关参考配置：

- [nginx.conf](../deploy/gateway/nginx.conf)：API、gRPC 与控制台三个站点，HSTS、HTTP→HTTPS 跳转、请求 ID 透传、登录与 API 速率限制。[test-gateway.sh](../scripts/test-gateway.sh) 在 CI 中用本地证书渲染该配置并验证 HTTP API 与 gRPC。
- [Caddyfile](../deploy/gateway/Caddyfile)：自动 ACME 证书的等价配置。

客户端连接 `https://` 地址时校验服务端证书。私有 CA 的信任方式：`gl --ca-file` / `GL_CA_FILE`、Rust `ConnectOptions::ca_pem`；Go 使用系统信任库或 `SSL_CERT_FILE`；Python 使用 `GRPC_DEFAULT_SSL_ROOTS_FILE_PATH`；Node.js（含控制台）使用 `NODE_EXTRA_CA_CERTS`。

### SDK 明文保护

四种 SDK 与 `gl` 只允许对回环地址（`localhost`、`127.0.0.0/8`、`::1`）使用 `http://`。连接其他主机的明文地址需要显式开启：Rust `ConnectOptions { allow_insecure: true }`、Go `DialOptions{AllowInsecure: true}`、Python `allow_insecure=True`、TS `{ allowInsecure: true }`、CLI `--allow-insecure`，或统一设置 `GL_ALLOW_INSECURE_TRANSPORT=true`。该开关只用于隔离的内部网络，例如同一 Compose 网络中的控制台到服务端。

## 用户与密码

用户身份由数据库中的稳定 UUID 标识，用户名去除首尾空白、按 ASCII 小写规范化并唯一，长度 3–64 字符，仅允许字母、数字、点、下划线和连字符；项目成员关系、历史与审计引用 UUID。平台管理员标志来自用户记录。用户禁用或软删除会使凭证与会话在下一次请求失效，同时保留内部身份和审计记录。

首次启动通过 `GL_ADMIN_USERNAME`（默认 `admin`）和可选 `GL_ADMIN_PASSWORD` 创建初始管理员。已有账户不会因重启重设密码。省略密码时使用系统随机源生成高强度密码，并以 create-new、0600 权限写入数据目录的 `initial-admin-<id>.json`；日志只提示路径。显式密码不写入该文件。初始密码和管理员重置的密码都必须先在 Console 修改。

密码保存为随机盐 Argon2id PHC 字符串（19 MiB、2 次迭代、1 并行度）。密码校验在有并发上限的阻塞任务中进行，不在数据库写锁内计算。登录失败采用统一语义，不存在的用户执行 dummy 密码校验。用户管理与凭证管理写操作，以及 Console 的项目成员权限修改，要求当前密码；自行改密也必须验证旧密码。

密码登录按去除首尾空白并转为 ASCII 小写的用户名限速，每服务进程允许突发 10 次、每 5 秒恢复一次；不存在的账号使用相同规则，不同账号拥有独立配额。匿名入口另按来源 IP 限制每秒 10 次、突发 100 次，保护同一 BFF 下的共享入口；API Key 登录仅使用这一粗粒度配额。默认不信任 X-Forwarded-For。已认证的敏感操作按稳定用户 UUID 独立限速，普通业务请求不消耗密码验证配额。密码计算最多同时运行 4 个任务；用户名桶仅保存规范化名称的摘要，限流表各最多 100000 条并有界清理。这些配额在各服务进程独立维护。

## API Key

Console 的“API Key”页面创建、查看、重命名、设置可选到期时间、撤销或删除密钥。密钥使用 `glk_` 加 32 字节随机数的十六进制编码；数据库只存 SHA-256 摘要。明文仅在创建响应展示一次。SDK 和 CLI 自动发送 `Authorization: Bearer <API Key>`，Key 不放入 URL 或命令行参数。

每个用户可以保留多个独立 Key，以便逐步轮换客户端。用户只管理自己的 Key；管理员管理其他用户也必须验证当前密码。Key 继承绑定用户的项目 RBAC，不获得额外数据权限。撤销、删除、过期或用户停用在后续请求强一致检查，已通过认证的在途操作可以完成。

CLI 通过 `GL_API_KEY` 或 `GL_API_KEY_FILE` / `--api-key-file` 读取 Key。文件是仅含一个 Key 的纯文本文件；Unix 下必须为私有普通文件（如 0600），符号链接和公开可读文件会被拒绝。

## PostgreSQL TLS

`GL_DATABASE_URL` 支持 libpq 的 TLS 参数，由 GeoLedger 构建证书校验器：

| sslmode | 加密 | 校验证书链 | 校验主机名 |
|---|---|---|---|
| verify-full（默认） | 必须 | 是 | 是 |
| verify-ca | 必须 | 是 | 否 |
| require | 必须 | 配置 `sslrootcert` 时校验 | 否 |
| disable | 否 | — | — |

- `prefer`、`allow` 会在服务器拒绝 TLS 时静默改用明文，启动时直接拒绝。
- `sslrootcert=<PEM 文件>` 只信任该 CA；`sslrootcert=system` 使用系统信任库。
- `sslcert` 与 `sslkey`（PKCS#8 PEM）提供客户端证书。
- `sslmode=disable` 面向回环地址或 Unix socket；连接其他主机时需要同时设置 `GL_DATABASE_ALLOW_PLAINTEXT=true`，仅用于隔离的私有网络。

示例：

```text
postgresql://geoledger@db.internal:5432/geoledger?sslmode=verify-full&sslrootcert=/etc/geoledger/db-ca.pem
```

`scripts/test-postgis-release.py` 在 CI 的 PostGIS 发布门禁中创建隔离数据库并实际执行 `crates/engine/tests/postgres_tls.rs`：仅允许 `hostssl` 的服务器验证 verify-full、verify-ca、require、错误 CA 和错误主机名被拒绝，以及非 TLS 服务器在默认与 require 模式下连接失败。受限角色和备份恢复演练也使用同一 TLS-only 服务。

## 控制台会话

浏览器 Cookie 仅持有 opaque Session（HttpOnly、SameSite=Strict、HTTPS 下 Secure）。Session 的摘要和来源保存在数据库，多实例共享状态。普通业务请求通过 Console 私有 HTTP transport 携带 Session；公开 SDK 仅接受 API Key。混合或重复凭证会被拒绝。

数据库 Session 空闲期限为 30 分钟、绝对期限为 12 小时，Console Cookie 最长 8 小时；退出登录在数据库撤销会话。密码修改、管理员重置、用户停用或删除立即使已有会话失效，并撤销该用户已有 API Key。通过 API Key 登录创建的会话持续绑定来源 Key，Key 过期、撤销或删除同时使会话失效。敏感管理操作独立验证当前密码，普通业务操作不重复要求密码。

完整数据库备份包含身份与凭证摘要；逻辑项目导入导出只处理业务数据。恢复旧备份可能恢复旧凭证状态，恢复后在对外服务前撤销或轮换相关 Key。TLS 私钥、初始化凭证文件及数据库备份均按秘密管理。

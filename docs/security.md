# 安全配置

[项目概览](../README.md) · [生产运行](production.md) · [开发验证](development.md)

本文说明传输加密、凭证生命周期、数据库 TLS 与控制台会话。所有设置均可通过环境变量或同名命令行参数提供。

## 传输加密

`geoledger-server` 可直接在 HTTP 与 gRPC 两个监听器上提供 TLS（rustls，TLS 1.2/1.3）：

| 环境变量 / 参数 | 用途 |
|---|---|
| GL_TLS_CERT / --tls-cert | PEM 证书链，与私钥同时配置后两个监听器都启用 TLS |
| GL_TLS_KEY / --tls-key | PEM 私钥（PKCS#8、PKCS#1 或 SEC1） |
| GL_TLS_CLIENT_CA / --tls-client-ca | PEM CA；配置后要求客户端证书（mTLS） |
| GL_RELOAD_INTERVAL_SECS | 证书、令牌与 JWKS 文件的变更检查间隔，默认 10 秒 |

HTTP 监听器协商 h2 与 http/1.1，gRPC 监听器协商 h2。证书文件更新后在检查间隔内生效，`kill -HUP` 立即重新加载；新文件校验失败时继续使用当前证书并记录错误日志。TLS 握手在独立任务中完成并有 10 秒期限。

监听非回环地址且未启用 TLS 时，服务启动会输出警告：此时请在可信网关终结 TLS。仓库提供两份网关参考配置：

- [nginx.conf](../deploy/gateway/nginx.conf)：API、gRPC 与控制台三个站点，HSTS、HTTP→HTTPS 跳转、请求 ID 透传、登录与 API 速率限制。[test-gateway.sh](../scripts/test-gateway.sh) 在 CI 中用本地证书渲染该配置并验证 HTTP API 与 gRPC。
- [Caddyfile](../deploy/gateway/Caddyfile)：自动 ACME 证书的等价配置。

客户端连接 `https://` 地址时校验服务端证书。私有 CA 的信任方式：`gl --ca-file` / `GL_CA_FILE`、Rust `ConnectOptions::ca_pem`；Go 使用系统信任库或 `SSL_CERT_FILE`；Python 使用 `GRPC_DEFAULT_SSL_ROOTS_FILE_PATH`；Node.js（含控制台）使用 `NODE_EXTRA_CA_CERTS`。

### SDK 明文保护

四种 SDK 与 `gl` 只允许对回环地址（`localhost`、`127.0.0.0/8`、`::1`）使用 `http://`。连接其他主机的明文地址需要显式开启：Rust `ConnectOptions { allow_insecure: true }`、Go `DialOptions{AllowInsecure: true}`、Python `allow_insecure=True`、TS `{ allowInsecure: true }`、CLI `--allow-insecure`，或统一设置 `GL_ALLOW_INSECURE_TRANSPORT=true`。该开关只用于隔离的内部网络，例如同一 Compose 网络中的控制台到服务端。

## 静态令牌

服务端令牌文件只保存 SHA-256 摘要：

```json
[
  {"subject": "alice", "token_sha256": "<64 位小写十六进制>", "expires_at": "2027-01-01T00:00:00Z", "label": "2026-10"},
  {"subject": "alice", "token_sha256": "<旧令牌摘要>", "disabled": true}
]
```

- `expires_at`（RFC 3339）到期后立即拒绝，无需重启。
- `disabled: true` 吊销条目并保留记录；删除条目同样立即生效。
- 同一 subject 可以有多个令牌，用于轮换期间新旧并存。
- 文件变更在检查间隔内自动加载，`SIGHUP` 立即加载；格式错误的新文件被拒绝，当前凭证继续有效。

生成凭证（明文只写入客户端文件或标准输出，服务端文件只含摘要）：

```sh
geoledger-server tokens --out ./server-tokens.json --client-out ./team-credentials.json \
  --expires-at 2027-01-01T00:00:00Z alice bob
```

客户端凭证文件格式为 `[{"subject": "...", "token": "..."}]`，权限 0600，交给对应用户后从服务器删除。手工计算摘要：`printf %s "$TOKEN" | sha256sum`。

服务端身份文件使用 `token_sha256` 摘要字段；`geoledger-server tokens` 同时生成服务端摘要文件和客户端凭证文件。

首次启动且数据目录没有令牌文件时，服务创建摘要格式的 `tokens.json` 和明文 `admin-credentials.json`（0600）。把后者交给运维人员后删除。生产环境可设置 `GL_BOOTSTRAP_ADMIN=false`，要求预先提供令牌文件。

## JWT 与 JWKS

JWT 只接受 RS256，校验签名、issuer（HTTPS）、audience、exp、可选 nbf 与 subject，JWKS 每个密钥都需要 `kid`。

| 环境变量 / 参数 | 用途 |
|---|---|
| GL_JWKS_FILE / --jwks-file | 本地 JWKS 文件，变更后自动重新加载 |
| GL_JWKS_URL / --jwks-url | HTTPS JWKS 地址，定时刷新 |
| GL_JWKS_REFRESH_SECS | URL 刷新间隔，默认 300 秒 |
| GL_JWT_ISSUER / GL_JWT_AUDIENCE | 固定 issuer 与 audience |

启动时必须成功获取一次 JWKS。之后刷新失败时保留上一份有效密钥并记录警告，响应体上限 1 MiB、请求期限 10 秒、不跟随重定向。身份提供方轮换密钥时，先发布新 `kid`，等待一个刷新周期后再签发新令牌。

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

`crates/engine/tests/postgres_tls.rs` 针对仅允许 `hostssl` 的服务器验证 verify-full、verify-ca、require、错误 CA 被拒绝，以及非 TLS 服务器在默认与 require 模式下连接失败。

## 控制台会话

浏览器 Cookie（HttpOnly、SameSite=Strict、HTTPS 下 Secure，iron-session 加密）只保存随机会话 ID，GeoLedger 凭证保存在控制台服务端内存。退出登录立即删除服务端会话，复制的 Cookie 随即失效；后端返回未认证时同样删除会话。会话绝对期限 8 小时，空闲期限由 `GL_WEB_SESSION_IDLE_MINUTES` 设置（默认 60 分钟）。控制台重启会让所有会话重新登录；多实例部署使用会话粘滞。

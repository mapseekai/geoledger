# 开发与验证

[项目概览](../README.md) · [存储接口](storage.md)

开发者在仓库根目录构建 Cargo 工作区，Web 使用独立 npm 项目。安装工具与验证版本见 [环境准备](getting-started.md#环境准备)，本地服务和热更新控制台分别见 [快速开始](getting-started.md) 与 [Web 文档](../web/README.md#本地启动)。

## 代码结构

| 目录 | 职责 |
|---|---|
| crates/core | 纯属性/几何三方合并 |
| crates/engine | 共同应用层、存储接口、SQLite/PostGIS 适配器 |
| crates/rpc / proto | 生成的 Rust RPC 类型与统一 Protobuf |
| crates/server | 存储业务服务、HTTP、tonic、认证、监控 |
| sdk/rust、sdk/go、sdk/ts、sdk/python | 四语言 gRPC 客户端 |
| crates/cli | 基于 Rust SDK 的远程命令行 |
| web | 独立 Next.js 控制台、服务端 TS SDK 与浏览器 HTTP |

## 构建与检查

构建 debug 服务端、CLI 和 Rust SDK 示例，再运行仓库检查：

```sh
cargo build --locked --workspace --bins --examples
./scripts/check.sh
```

`scripts/check.sh` 使用 Python 3 和 Bash，依次执行文档检查、rustfmt、Clippy、独立引擎单元测试和工作区测试。完整检查中的真实服务用例会监听临时本地端口。

PostGIS 回归通过环境中的 `GL_TEST_DATABASE_URL` 启用，目标库名为 `geoledger_test`。测试服务账号需要对应隔离库的建表和事务权限。Web 检查命令见 [Web 构建与测试](../web/README.md#构建测试与部署)。

生产构建：

```sh
cargo build --release --locked --bins
```

## 生成 SDK

SDK 业务层手工维护，公开接口提供业务客户端、业务模型和业务错误。Rust 内部绑定由 Cargo build 从 proto 生成；其余语言的内部生成结果提交在源码中；修改 proto 后执行 [generate-sdk.sh](../scripts/generate-sdk.sh)。工具版本：protoc 3.21.12、protoc-gen-go 1.36.10、protoc-gen-go-grpc 1.5.1、grpcio-tools 1.78.0、ts-proto 2.13.0。

```sh
go install google.golang.org/protobuf/cmd/protoc-gen-go@v1.36.10
go install google.golang.org/grpc/cmd/protoc-gen-go-grpc@v1.5.1
python3 -m venv .venv
. .venv/bin/activate
python -m pip install grpcio-tools==1.78.0
export PATH="$(go env GOPATH)/bin:$PATH"
npm ci --prefix sdk/ts
./scripts/generate-sdk.sh
```

生成工具需在 PATH 上。Python 脚本可用 `PYTHON` 指定含 grpcio-tools 的解释器。Go 模块要求 Go 1.25+；TypeScript 构建/运行推荐 Node.js 22；Python 客户端要求 3.10+。

## 四语言联调

启动一个隔离服务，设置 `GL_ENDPOINT` 和 `GL_TOKEN_FILE`，运行 [test-sdks.sh](../scripts/test-sdks.sh)。令牌通过私有文件读取，脚本通过公开业务接口创建独立项目、保存精确数字、发布、重复原请求和读回。分别对 SQLite/PostGIS 地址运行。

在一个终端启动专用服务：

```sh
./target/debug/geoledger-server --data-dir ./target/sdk-test
```

另一个终端准备 Python 环境、TS 依赖并运行联调（Go 1.25+ 已在 PATH）：

```sh
python3 -m venv .venv
. .venv/bin/activate
python -m pip install ./sdk/python
npm ci --prefix sdk/ts
export GL_ENDPOINT=http://127.0.0.1:7882
export GL_TOKEN_FILE="$PWD/target/sdk-test/admin-credentials.json"
./scripts/test-sdks.sh
```

SDK 示例与测试会写入新的测试项目，请使用专用测试服务。CLI 完全通过 Rust SDK 访问 gRPC；安装 CLI 后即可通过服务地址和凭证操作数据。

## 测试原则

新后端接入必须保持 [存储语义](storage.md)，运行同样的 conformance 测试；全部后端保持相同场景覆盖。单独测试存储期限、提交失败、持久化重启和发布幂等。

容量测试需要显式启用并记录环境、数据规模、几何类型、索引、请求模型和耗时分位数。分别记录 SQL 直接播种和公共接口导入的验证结果；生产 SLA 通过目标环境中的专项容量与延迟验收确定。

## 回归测试入口

| 入口 | 验证内容 |
|---|---|
| [存储一致性](../crates/engine/tests/conformance.rs) | SQLite/PostGIS 的授权、并发编辑、发布幂等、合并冲突、历史与分页语义 |
| [事务与持久化](../crates/engine/tests/durability.rs) | SQLite 锁等待期限、事务回滚、原请求重试与重启读取 |
| [存储契约](../crates/engine/tests/storage.rs) | 后端事务接口与不可变记录约束 |
| [真实服务](../crates/server/tests/service.rs) | HTTP/gRPC、身份校验、并发发布与重启 |
| [冲突分页](../crates/server/tests/rpc_errors.rs) | HTTP/2 错误摘要与完整冲突分页查询 |
| [多实例并发](../crates/engine/tests/multi_instance.rs) | 多个实例（各自连接池）共享 SQLite/PostGIS：无交集的发布全部落地且修订号唯一，原请求跨实例重试幂等，重叠编辑恰有一个胜出 |
| [导出与备份](../crates/engine/tests/portable.rs) | 导出导入往返、损坏文件拒绝、SQLite 在线备份、SQLite ⇄ PostGIS 互通 |
| [失败路径](../crates/server/tests/limits.rs) | 外部锁超过请求期限、执行槽耗尽（429 / RESOURCE_EXHAUSTED）、请求体停滞（408）、畸形凭证（401），失败后恢复 |
| [运维接口](../crates/server/tests/operations.rs)、[传输安全](../crates/server/tests/security.rs) | request ID 与指标、限流、排空与停机；TLS/mTLS、令牌轮换、JWKS 刷新 |
| [Rust SDK](../sdk/rust/tests/client.rs) | 发布结果确认与原请求恢复 |
| [Go SDK](../sdk/go/client_test.go)、[Python SDK](../sdk/python/tests/test_client.py)、[TS SDK](../sdk/ts/test/client.test.cjs) | JSON 精度、显式编辑、工作区版本与发布重试 |
| [四语言联调](../scripts/test-sdks.sh) | SDK 连接实际服务完成创建、编辑、发布与读取 |

## 容量验证

[容量测试](../crates/engine/tests/capacity.rs) 在百万 Point 要素上运行 20 个独立身份的分页查询、草稿保存、发布、原请求重试与读取，并核对版本和历史一致性。

```sh
cargo test --locked -p geoledger-engine --test capacity -- --ignored --nocapture
GL_CONFORMANCE_BACKEND=postgis cargo test --locked -p geoledger-engine --test capacity -- --ignored --nocapture
```

SQLite 用例使用临时目录并自动清理；PostGIS 用例使用 `GL_TEST_DATABASE_URL` 指定的隔离 `geoledger_test` 数据库。准备数 GB 可用空间。用例通过 SQL 播种建立数据集，操作耗时从播种完成后开始测量；公共接口导入、复杂几何和长期运行按目标业务模型单独验收。

## 模糊测试与负载

[编解码与几何](../crates/engine/src/fuzzing.rs) 的性质检查（解析→编码→再解析一致、几何规范化稳定、边界与相交计算不报错）在 `cargo test` 中以确定性变异样本运行；[cargo-fuzz 目标](../fuzz/Cargo.toml) 用 nightly 工具链做覆盖率引导的长时间运行：

```sh
cargo install cargo-fuzz
cd fuzz
cargo +nightly fuzz run codec -- -max_total_time=300
cargo +nightly fuzz run geometry -- -max_total_time=300
```

发现的崩溃样本写入 `fuzz/artifacts/`，修复后把最小化样本加入对应的单元测试。

[负载脚本](../scripts/load-test.sh) 启动临时服务，用 [k6](https://grafana.com/docs/k6/latest/) 运行 [HTTP 负载模型](../scripts/load/http.js)：每次迭代创建工作区、保存一批点要素、发布并做范围查询。脚本在 p95 延迟、失败率或繁忙率超过阈值，服务端出现内部错误，或已发布版本数与 k6 统计的成功发布数不一致时失败：

```sh
GL_LOAD_VUS=20 GL_LOAD_DURATION=60s ./scripts/load-test.sh
GL_DATABASE_URL='postgresql://...' GL_DATABASE_ALLOW_PLAINTEXT=true ./scripts/load-test.sh
```

阈值可用 `GL_LOAD_P95_READ_MS`、`GL_LOAD_P95_WRITE_MS` 调整；`GL_LOAD_SUMMARY` 保存 k6 汇总 JSON 供趋势对比。[nightly 工作流](../.github/workflows/nightly.yml) 每天运行容量测试、两种后端的负载脚本和两个 fuzz 目标。

## 控制台浏览器验证

[控制台说明](console.md) 包含登录、资源管理和发布结果确认与恢复流程。
Web 的安装、构建和单元检查见 [web/README.md](../web/README.md)。
真实浏览器验证需要 Python Playwright 和 Chromium：

```sh
python3 -m venv .venv
. .venv/bin/activate
python -m pip install playwright
python -m playwright install chromium
# 在独立终端用 --data-dir ./target/console-test 启动后端，按 Web 文档配置并启动控制台后：
python3 scripts/test-console.py \
  --url http://localhost:3000 \
  --token-file target/console-test/admin-credentials.json \
  --screenshots artifacts
```

也可用 `--chromium /usr/bin/chromium` 指定已安装浏览器。
测试会写入新项目，验证登录、CSRF、精确数字、断线后原请求重试、分页、
撤销、权限、审计、退出与手机布局。只对隔离测试服务运行。
前端检查独立于 Rust 的 `check.sh`，CI 单独安装依赖并执行 Web 检查。

截图由 `--screenshots artifacts` 生成：`artifacts/next-login.png`、`artifacts/next-projects.png`、`artifacts/next-mobile.png` 和 `artifacts/next-login-mobile.png`。这些是本地生成产物。

## 依赖与发布

[audit.sh](../scripts/audit.sh) 是依赖安全门禁（需要网络）：`cargo audit --deny warnings`（RustSec，已接受的公告和理由写在 [.cargo/audit.toml](../.cargo/audit.toml)）、`web/` 与 `sdk/ts/` 的 `npm audit`（运行时依赖 moderate 及以上、全部依赖 high 及以上）、`sdk/go` 的 `govulncheck`、`sdk/python` 的 `pip-audit --strict`。后两个工具未安装时跳过，设置 `GL_AUDIT_REQUIRE_ALL=1` 时视为失败（CI 设置）。`govulncheck` 按运行它的 Go 版本判断标准库公告，使用当前受支持的 Go 版本运行。

[Dependabot](../.github/dependabot.yml) 每周为 Cargo、npm（web、sdk/ts）、Go、pip、GitHub Actions 与 Dockerfile 基础镜像提出更新。Actions 固定到完整 commit SHA 并在注释中标注版本；Dockerfile 基础镜像固定 digest，由 Dependabot 刷新。

发布流程：更新 `Cargo.toml`、`sdk/ts/package.json`、`sdk/python/pyproject.toml`（PEP 440 形式，如 `0.3.0a1`）的版本，把 [CHANGELOG](../CHANGELOG.md) 的 `Unreleased` 段落改为同名版本，合并后在 main 上推送 `vX.Y.Z` 标签。发布 workflow（`.github/workflows/release.yml`）校验标签与各包版本一致，构建 Linux x86_64/aarch64 与 Windows x86_64 二进制（含 SHA256、源码 SBOM 和构建来源证明）、多架构服务与控制台镜像（推送到 GHCR，附 SBOM 与 provenance，cosign 无密钥签名），创建 GitHub Release，并在配置凭证时发布 npm/PyPI SDK、为 Go 模块打 `sdk/go/vX.Y.Z` 标签。协议兼容性由 CI 的 `buf breaking` 对比目标分支检查，规则见 [兼容性与弃用](api.md#兼容性与弃用)；漏洞处理见 [安全策略](../SECURITY.md)。

## 贡献流程

1. 在 [GitHub Issues](https://github.com/mapseekai/geoledger/issues) 描述需求或问题，提供服务版本、存储后端、复现步骤和期望结果；日志使用脱敏内容。
2. 获取仓库，阅读 [AGENTS.md](../AGENTS.md)，创建针对单个问题的工作分支。
3. 保持 core、Application、存储适配器和传输层的职责；业务操作统一经过 Application，SQL 封装在 session 适配器。
4. 为行为修改增加对应回归，运行 `scripts/check.sh`；Web 修改完成 Web 检查及相关浏览器场景，PostGIS 修改完成隔离库回归。
5. 提交 Pull Request，说明问题、最终行为、验证命令与范围。主 [CI](../.github/workflows/ci.yml) 覆盖 Linux、Windows、Rust 最低版本、Web、双后端 SDK 联调、TLS 网关、依赖审计与 proto 兼容性。

协议以 `proto/geoledger/v1/geoledger.proto` 为统一来源，生成绑定按本页工具版本更新。存储结构变化使用显式格式版本和前向迁移（`geoledger-server migrate`），格式与恢复要求见 [存储接口](storage.md)。文档保持主题集中、仓库相对链接与当前行为说明；项目使用 [MIT 许可证](../LICENSE)。

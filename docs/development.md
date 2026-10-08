# 开发与验证

[项目概览](../README.md) · [存储接口](storage.md)

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

构建依赖 Rust 1.88+（推荐 1.92）、C 编译器、protoc、pkg-config、OpenSSL 开发库。SQLite 后端随二进制提供，可直接运行。

```sh
cargo build --locked --workspace --bins --examples
./scripts/check.sh
```

常规检查包括文档链接、fmt、Clippy、Rust 单元、SQLite 一致性、事务失败与真实服务测试。引擎单元测试还会独立运行，验证默认 serde_json 特性配置下的行为。DB 测试仅使用隔离的 `geoledger_test` 数据库；设置 `GL_TEST_DATABASE_URL` 后会追加 PostGIS 的相同 conformance 套件、审计提交顺序并发测试和存储测试。

## 生成 SDK

SDK 业务层手工维护，公开接口提供业务客户端、业务模型和业务错误。Rust 内部绑定由 Cargo build 从 proto 生成；其余语言的内部生成结果提交在源码中；修改 proto 后执行 [generate-sdk.sh](../scripts/generate-sdk.sh)。工具版本：protoc 3.21+、protoc-gen-go 1.36.10、protoc-gen-go-grpc 1.5.1、grpcio-tools 1.78.0、ts-proto 2.11.0。

```sh
go install google.golang.org/protobuf/cmd/protoc-gen-go@v1.36.10
go install google.golang.org/grpc/cmd/protoc-gen-go-grpc@v1.5.1
python -m pip install grpcio-tools==1.78.0
npm ci --prefix sdk/ts
./scripts/generate-sdk.sh
```

生成工具需在 PATH 上。Python 脚本可用 `PYTHON` 指定含 grpcio-tools 的解释器。Go 模块要求 Go 1.23+；TypeScript 构建/运行推荐 Node.js 22；Python 客户端要求 3.10+。

## 四语言联调

启动一个隔离服务，设置 `GL_ENDPOINT` 和 `GL_TOKEN_FILE`，运行 [test-sdks.sh](../scripts/test-sdks.sh)。令牌通过私有文件读取，脚本通过公开业务接口创建独立项目、保存精确数字、发布、重复原请求和读回。分别对 SQLite/PostGIS 地址运行。

```sh
export GL_ENDPOINT=http://127.0.0.1:7882
export GL_TOKEN_FILE="$PWD/geoledger-data/tokens.json"
./scripts/test-sdks.sh
```

SDK 示例与测试会写入新的测试项目，请使用专用测试服务。CLI 完全通过 Rust SDK 访问 gRPC；安装 CLI 后即可通过服务地址和凭证操作数据。

## 测试原则

新后端接入必须保持 [存储语义](storage.md)，运行同样的 conformance 测试；全部后端保持相同场景覆盖。单独测试存储期限、提交失败、持久化重启和发布幂等。

容量测试需要显式启用并记录环境、数据规模、几何类型、索引、请求模型和耗时分位数。分别记录 SQL 直接播种和公共接口导入的验证结果；生产 SLA 通过目标环境中的专项容量与延迟验收确定。

### 2026-10-08 格式 5 修复验证

本地 macOS arm64、Rust 1.95 nightly、Node.js 22、Python 3.12 环境完成带隔离 PostGIS 库的 `scripts/check.sh`：SQLite/PostGIS 各 15 个一致性用例通过，包括每条含 7,000 个数组元素的 1,000 条要素发布及冲突分页。原有超时和测试规模保持不变；审计提交顺序、事务回滚、服务重启和存储契约检查通过。

四语言 SDK 的单元测试及 `test-sdks.sh` 在两个后端通过。Web 单元测试、类型检查、格式检查和生产构建通过；真实浏览器验证覆盖登录、项目创建、弹窗关闭与重新打开、表格筛选和退出。另用独立 Rust crate 验证默认、`raw_value`、`arbitrary_precision` 特性组合下的数字与保留键行为。本次验证范围为上述正确性回归与浏览器交互场景；百万要素容量基线见 [容量记录](production-review.md#容量方法)，完整浏览器套件的执行入口见下节。生产延迟指标通过目标环境专项验收确定。

## 控制台浏览器验证

[控制台说明](console.md) 包含登录、资源管理和失败恢复流程。
Web 的安装、构建和单元检查见 [web/README.md](../web/README.md)。
真实浏览器验证需要 Python Playwright 和 Chromium：

```sh
python3 -m pip install playwright
python3 -m playwright install chromium
# 启动隔离的 geoledger-server 与已配置的 Web 项目后：
python3 scripts/test-console.py \
  --url http://localhost:3000 \
  --token-file target/console-test/tokens.json \
  --screenshots artifacts
```

也可用 `--chromium /usr/bin/chromium` 指定已安装浏览器。
测试会写入新项目，验证登录、CSRF、精确数字、断线后原请求重试、分页、
撤销、权限、审计、退出与手机布局。只对隔离测试服务运行。
前端检查独立于 Rust 的 `check.sh`，CI 单独安装依赖并执行 Web 检查。

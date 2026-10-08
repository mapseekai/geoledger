# 开发与验证

[项目概览](../README.md) · [存储接口](storage.md)

## 代码结构

| 目录 | 职责 |
|---|---|
| crates/core | 纯属性/几何三方合并 |
| crates/engine | 共同应用层、存储接口、SQLite/PostGIS 适配器 |
| crates/rpc / proto | 生成的 Rust RPC 类型与统一 Protobuf |
| crates/server | 唯一服务程序、HTTP、tonic、认证、监控、内置 UI |
| sdk/rust、sdk/go、sdk/ts、sdk/python | 四语言 gRPC 客户端 |
| crates/cli | 基于 Rust SDK 的远程命令行 |

构建依赖 Rust 1.88+（推荐 1.92）、C 编译器、protoc、pkg-config、OpenSSL 开发库。运行 SQLite 后端无需数据库或空间扩展安装。

```sh
cargo build --locked --workspace --bins --examples
./scripts/check.sh
```

常规检查包括文档链接、浏览器数值/重试、fmt、Clippy、Rust 单元、SQLite 一致性、事务失败与真实服务测试。DB 测试仅使用隔离的 `geoledger_test` 数据库；设置 `GL_TEST_DATABASE_URL` 后会追加 PostGIS 的相同 conformance 套件。

## 生成 SDK

SDK 业务层手工维护，公开接口不暴露生成类型。Rust 内部绑定由 Cargo build 从 proto 生成；其余语言的内部生成结果提交在源码中；修改 proto 后执行 [generate-sdk.sh](../scripts/generate-sdk.sh)。工具版本：protoc 3.21+、protoc-gen-go 1.36.10、protoc-gen-go-grpc 1.5.1、grpcio-tools 1.78.0、ts-proto 2.11.0。

```sh
go install google.golang.org/protobuf/cmd/protoc-gen-go@v1.36.10
go install google.golang.org/grpc/cmd/protoc-gen-go-grpc@v1.5.1
python -m pip install grpcio-tools==1.78.0
npm ci --prefix sdk/ts
./scripts/generate-sdk.sh
```

生成工具需在 PATH 上。Python 脚本可用 `PYTHON` 指定含 grpcio-tools 的解释器。Go 模块要求 Go 1.23+；TypeScript 构建/运行推荐 Node.js 22；Python 客户端要求 3.10+。

## 四语言联调

启动一个隔离服务，设置 `GL_ENDPOINT` 和 `GL_TOKEN_FILE`，运行 [test-sdks.sh](../scripts/test-sdks.sh)。令牌不会打印到终端，脚本通过公开业务接口创建独立项目、保存精确数字、发布、重复原请求和读回。分别对 SQLite/PostGIS 地址运行。

```sh
export GL_ENDPOINT=http://127.0.0.1:7882
export GL_TOKEN_FILE="$PWD/geoledger-data/tokens.json"
./scripts/test-sdks.sh
```

SDK 示例与测试会写入新的测试项目，请使用专用测试服务。CLI 完全通过 Rust SDK 访问 gRPC；安装 CLI 不需要数据库驱动。

## 测试原则

新后端接入必须保持 [存储语义](storage.md)，运行同样的 conformance 测试；禁止通过删除场景规避后端差异。单独测试存储期限、提交失败、持久化重启和发布幂等。

容量测试需要显式启用并记录环境、数据规模、几何类型、索引、请求模型和耗时分位数。SQL 直接播种与经公共接口导入属于不同验证范围；不要将正确性回归时间或某次小样本测量当作生产 SLA。

## 控制台浏览器验证

[控制台说明](console.md) 包含登录、资源管理和失败恢复流程。
常规检查中的 Node 测试覆盖精确数值、原请求重试和会话结束后的迟到响应。
真实浏览器验证使用 [test-console.py](../scripts/test-console.py)，需要 Python Playwright 和 Chromium：

```sh
python3 -m pip install playwright
python3 -m playwright install chromium
# 在另一终端启动隔离服务：
# target/debug/geoledger-server --data-dir target/console-test --http 127.0.0.1:7895 --grpc 127.0.0.1:7896
python3 scripts/test-console.py \
  --url http://127.0.0.1:7895 \
  --token-file target/console-test/tokens.json \
  --screenshots artifacts
```

也可用 `--chromium /usr/bin/chromium` 指定已安装的浏览器。
脚本会创建测试项目和数据，验证无效令牌、完整编辑发布流程、大整数、
丢失发布响应后的幂等重试、分页、权限设置、会话清理与移动端布局。
截图不包含令牌明文。只在隔离测试服务上运行；此项因依赖浏览器，不纳入默认 `check.sh`。

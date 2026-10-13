# 大数据与压力测试

本指南使用独立的服务、数据库和临时端口，测量真实 GeoJSON 从客户端导入、发布到查询回读的全过程。原始文件只读；测试生成的项目、数据库、凭据和报告位于显式指定的输出目录。PostGIS 测试创建专用 Docker 容器，结束后自动删除。

## 已有大数据集上的增量增删改

`incremental` Rust RPC 客户端测量已有数据集上的 10、100、500 条新增、修改和删除，分别记录保存、发布、原请求重试及回读的 p50/p95/p99。`added` 场景每轮先添加独立 ID 的要素，再修改这些要素的属性和 XY 坐标，最后删除；`existing` 场景读取背景数据中间的连续记录，执行修改、删除、原样恢复。每次发布后核对当前版本和前一版本，完整循环后核对初始快照。XYZ 要素的 Z 值保持一致，背景夹具至少包含 500 条要素。500 条保存按服务批次上限拆成五次 RPC，同时报告单次和整批耗时。

```sh
cargo build --locked --release -p geoledger-server -p geoledger-client \
  --bin geoledger-server --example incremental --example fixture_jsonl
mkdir -p artifacts/performance/fixtures
target/release/examples/fixture_jsonl /absolute/path/building.geojson \
  artifacts/performance/fixtures/building.jsonl EPSG:4326 \
  > artifacts/performance/fixtures/building.meta.json
python3 scripts/benchmark_incremental.py \
  --server target/release/geoledger-server \
  --client target/release/examples/incremental \
  --fixture artifacts/performance/fixtures/building.jsonl \
  --metadata artifacts/performance/fixtures/building.meta.json \
  --output artifacts/performance/incremental-sqlite --rounds 10
```

### 夹具坐标系与内部几何列

`fixture_jsonl SOURCE OUTPUT CRS` 将显式指定的源坐标系写入夹具元数据，例如 `EPSG:4326`、`EPSG:3857` 或自定义 CRS 定义。参数应与源文件的实际坐标系一致；生成器保留原始坐标。已有 JSONL 夹具可通过 `benchmark_incremental.py --crs EPSG:4326` 补充经过核对的 CRS。监督器在启动前校验 CRS 元数据及显式参数的一致性。

当前格式 14 将数据集 CRS 保存在 `gl_datasets.crs`，SQLite 和 PostGIS 的共享内部几何列使用 SRID 0。SQLite 的 XY/XYZ 分别写入 `geom`/`geom_z`；PostGIS 业务表按照 `EPSG:<编号>` 或 `POSTGIS:<编号>` 声明的 SRID 创建，历史表仍使用内部 SRID 0。两条写入路径均保留坐标数值。初始化先校验当前存储格式，SQLite 还校验几何列的维度及 SRID；运行结束后核对历史表与业务表的实际 SRID。`environment.json` 和 `result.json` 记录 CRS、内部/业务 SRID 及执行阶段。

增量客户端与原表校验器在数据集原生坐标空间中执行可表示的 XY 平移，保留几何类型、环闭合和 Z 值。经纬度与投影坐标使用同一增量验证流程。功能诊断可使用 debug 构建；性能对照使用本次代码构建的 release 二进制，并核对服务版本及报告中的构建摘要。

SQLite 设置可信的 `GL_SPATIALITE_EXTENSION`；加 `--backend postgis` 测量已有业务表的同步发布。夹具将原属性保存在 `attributes` 对象中，业务表使用 `id` 主键、`attributes jsonb` 和有明确类型的 `geom` 列；业务现场额外的索引、约束与触发器可用独立环境进一步测量。监督器通过 API 建立初始版本，在自己创建的一次性数据库中用 SQL 填充背景夹具，然后运行纯 RPC 客户端。准备夹具的耗时单独记录。PostGIS 测量结束后另做一个未计时增删改循环，每一步直接读取原表核对完整属性和几何，再核对原表与版本表的存活要素数量。对比修复前后时，用 `--server` 指定各自的 release 二进制，保持同一份夹具、客户端及轮数，顺序运行负载。

输出包含构建与夹具 SHA-256、平台、逐次延迟、分位数、历史/幂等断言、进程 RSS 和清理状态。默认 10 轮的每组分位数适合本机回归对照；持续负载评估可增加轮数，并结合下方的固定到达率与并发测试。添加 `--k6 /absolute/path/k6`，可在增量循环后对同一背景数据施加 50 请求/秒、持续 30 秒的首页、末页、单要素和空范围查询负载，阈值与下方读取压力测试一致。业务事务变化通过 `./scripts/check.sh` 的一致性与 PostGIS 触发器回归验证。

## Rust RPC 客户端

```sh
cargo build --locked --release -p geoledger-server -p geoledger-client \
  --bin geoledger-server --example large_data
python3 scripts/benchmark.py \
  --geojson /absolute/path/building.geojson \
  --geojson /absolute/path/geobig_4326.geojson \
  --output artifacts/performance/sqlite-run
```

SQLite 使用服务配置的可信 SpatiaLite 扩展路径 `GL_SPATIALITE_EXTENSION`。添加 `--backend postgis` 可运行相同 RPC 客户端，并额外验证已有业务表纳管及单行发布写回。输出目录每次使用新路径。

客户端流式解析 FeatureCollection，使用容量为 2 的批次队列。每批最多 100 个要素或约 1 MiB，单个较大要素独立传输。为按文件顺序分页校验，测试数据集使用补零序号作为要素 ID，保留原始属性和几何。摘要按照服务的数字语义比较：整数属性保留精度，几何按双精度坐标比较，`3` 与 `3.0` 等价。

一次测试包含：

- 全文件分批保存与初始发布，以及同一发布请求重放。
- 固定版本完整分页回读，核对数量、顺序和 SHA-256 摘要。
- 删除与撤销恢复，核对原始要素。
- 1、4、20 路并发查询，每路执行 20 轮首页、末页、空 bbox 和单要素查询。
- PostGIS 业务表初始纳管和单行发布，并直接查询原表验证写回。

报告记录导入和发布耗时、回读耗时、各操作 p50/p95/p99、最大延迟、错误码、进程 RSS 及测试构建摘要。RPC 查询采用固定并发模型，延迟升高会降低请求速率。导入时间包含客户端解析与序列化；完整回读时间包含摘要校验。PostGIS 容器资源另存为 `postgres-resources.jsonl`。

## HTTP 压力测试

将本机 k6 路径传给监督脚本：

```sh
python3 scripts/benchmark.py --geojson /absolute/path/building.geojson \
  --output artifacts/performance/http-run --k6 /absolute/path/k6
```

`k6/scripts/large-data-reads.js` 对已导入的数据分别施加 5、20、50 请求/秒，每档 30 秒。固定到达率用于观察请求积压；报告同时包含失败率、丢弃迭代及各查询延迟。默认诊断阈值为 p95 小于 500 ms、请求失败率低于 1%、检查通过率高于 99% 和零丢弃迭代。阈值超出时保留失败报告。

并发写入、发布和空间查询使用仓库现有客户端：

```sh
GL_LOAD_VUS=20 GL_LOAD_DURATION=60s GL_SERVER_BIN=target/release/geoledger-server \
  GL_LOAD_SUMMARY=artifacts/performance/geoledger-write-load.json ./scripts/load-test.sh
```

## 功能与内存验证

```sh
./scripts/check.sh
cargo test --locked -p geoledger-client --example large_data
python3 -m unittest discover -s scripts -p 'test_benchmark*.py'
cargo +nightly miri test -p geoledger-core
cargo +nightly fuzz run geometry -- -max_total_time=60 -rss_limit_mb=1024
cargo +nightly fuzz run codec -- -max_total_time=60 -rss_limit_mb=1024
```

全仓库检查覆盖业务一致性、授权、事务回滚、幂等发布与备份恢复；`GL_TEST_DATABASE_URL` 指向专用 `geoledger_test` 数据库时加入 PostGIS 回归。核心合并的状态组合测试可以在 Miri 下运行；几何和 JSON fuzz 目标使用 AddressSanitizer。macOS 监督脚本会额外执行服务进程的原生 `leaks` 检查。

真实浏览器大文件流程见 [Web 大文件验证](../web/README.md)。对已经启动的独立测试 Console，可使用文件性能客户端：

```sh
python3 scripts/test-console-file-performance.py \
  --url http://127.0.0.1:3000 --token-file /temporary/data/admin-credentials.json \
  --chromium /absolute/path/chrome --geojson /absolute/path/building.geojson \
  --expected-features 251014 --report artifacts/performance/browser-building.json
```

Python 环境需安装 Playwright。客户端通过创建数据集对话框选择本地文件，核对成功保存的要素数量，记录上传耗时、首次地图显示、浏览器进程合计 RSS、页面错误和截图。默认上传期限为 1200 秒，浏览器 RSS 上限为 4096 MiB。首次地图显示只衡量当前页面的预览；完整数据一致性由 Rust 客户端的逐页摘要校验验证。浏览器上传、地图绘制和 RPC 吞吐分别记录，以便定位具体阶段。

源表独立校验按 1、100、500 条分别执行新增、属性和几何修改、删除；每次发布后直接查询 PostGIS 业务表，逐条核对完整内容与数量。校验在计时阶段之外执行，测试监督器还覆盖仅写入批次第一条、遗漏修改或删除、几何未更新及数字精度丢失等故障。

## 测量条件

监督脚本默认将 RPC 和服务操作期限设为 600 秒，单项客户端总期限为 1200 秒；这些值保存在 `environment.json`。每 0.5 秒采样服务与客户端 RSS，单进程超过 4096 MiB 时停止该项测试。RSS 采样用于观察内存规模和趋势；Miri、模糊测试与原生泄漏检查提供各自覆盖范围内的证据。

使用 release 构建，记录硬件、版本、文件规模、构建摘要、运行参数及错误。测量期间按顺序运行负载，结果用于这台机器上的工作负载比较。生产容量评估还需在目标硬件、网络、数据分布与长时间持续负载下测量。

## 格式 13 增量编辑实测（2026-10-11）

在 Apple Silicon 10 核、16 GiB 内存上，以格式 13 的 release 服务、本机 SQLite/SpatiaLite 和 Docker PostgreSQL 17 / PostGIS 3.6 测试。背景为 483,268 条真实 XYZ 线要素，夹具约 999 MiB。每组 20 轮，500 条拆成 5 次保存 RPC 后发布；背景准备时间单独记录。此机器同时运行其他开发服务，数据代表本机本次运行。

PostGIS 为 `0.3.0-alpha.2` 最终候选二进制，功能检查结束后开始性能计时。SQLite 保留同日上一候选构建的最近性能记录；两个后端均已通过当前代码的功能检查与四语言 SDK 联调。

下表按新增、修改、删除汇总，每个批量大小 100 个样本，单位毫秒。整批耗时为保存加首次发布；原请求重放与回读另记。

| 后端 | 批量 | p95 | p99 | 最大值 |
|---|---:|---:|---:|---:|
| SQLite | 10 | 52 | 78 | 210 |
| SQLite | 100 | 199 | 311 | 560 |
| SQLite | 500 | 1348 | 4106 | 4217 |
| PostGIS alpha.2 | 10 | 111 | 201 | 545 |
| PostGIS alpha.2 | 100 | 598 | 1115 | 1918 |
| PostGIS alpha.2 | 500 | 4939 | 8018 | 8081 |

两个后端各完成 360 次顺序发布、原请求重放及当前值和历史值核对，计时阶段分别约 146 秒、298 秒。服务 RSS 峰值分别为 103.64 MiB、30.81 MiB。原表夹具使用文本主键、attributes JSONB 与类型化 geom；额外业务索引、约束、触发器和网络需要在目标部署环境测量。

### 并发发布验收

当前压测分别统计保存、首次发布和收据重放，并对每个操作及批量分组应用 p95 ≤ 5000 ms 门限，快速重放不会稀释首次发布的延迟。PostGIS 的临时合并表使用数据集与要素键索引；已提交收据在成员与工作区权限校验后返回，首次发布保持项目写锁及锁内幂等性复查。

PostGIS alpha.2 以 20 路并发持续 300 秒，完成 404 个完整增删改循环、1212 次首次发布和 1212 次重放，错误数为 0：

| 操作 | p95（ms） | p99（ms） |
|---|---:|---:|
| 单次保存（最多 100 条） | 492 | 889 |
| 首次发布 | 5916 | 9824 |
| 收据重放 | 22 | 48 |
| 单要素回读 | 24 | 53 |

**性能验收待空闲环境复测，5 秒门限保持不变**：500 条新增、修改、删除发布分组 p95 分别为 6415、8731、8961 ms，部分较小批量分组也超出门限。服务 RSS 峰值为 55.61 MiB。宿主机采样显示持续大量换页，期间换入、换出速率达到每秒数十至逾百 MiB，因此本次结果用于保留实测证据，不能据此确定稳定容量或将变化单独归因于代码。已停止补充压测，保留修复及报告，待空闲环境使用同一负载重新验收。

延迟门限失败后，监督器继续完成 1/100/500 条原表新增、属性与几何修改、删除核对；原表和版本库的存活要素均为 483,268 条，完整性检查通过。原生内存扫描因调试权限受限记为 `error`，本次没有取得原生零泄漏结论。

SQLite 上一候选构建以 20 路并发持续 300 秒，完成 1538 个完整增删改循环、零错误，所有保存与发布分组通过 5 秒门限。保存总体 p95 为 813 ms，发布总体 p95 为 801 ms，单要素回读 p95 为 13 ms；服务 RSS 峰值为 125.31 MiB，结束后存活要素仍为 483,268 条。该旧报告将首次发布和重放合并统计，与当前 PostGIS 的分拆统计分别解读。

同日上一候选构建的独立 PostGIS 演练通过了受限角色、原生备份恢复、历史、审计、成员令牌恢复和 1/100/500 条新增、属性与几何修改、删除的逐项原表校验，并通过真实浏览器纳管、编辑和发布。小夹具恢复用时约 0.96 秒，正式数据库的恢复时限按实际备份体量验收。

本机证据位于 `artifacts/performance/alpha2-postgis-batched-final`、`artifacts/performance/release13-sqlite-after`，分别保留二进制和夹具摘要、环境、逐次延迟及 RSS。测试数据库、凭据和日志不提交。原生内存检查的调试权限受限时记录为 `error`，RSS 和功能通过分别报告。

## macOS 原生内存信号诊断

原生检查结合分配栈、同一进程的重复快照和对象释放行为进行判断。`leaks` 通过扫描可达指针识别内存；`BytesMut` 消费数据后，可以用未对齐的内部指针和编码偏移保留原分配的所有权。该状态在 macOS 15.6.1、bytes 1.12.1 上能够产生 96 KiB 的 `ROOT LEAK` 信号，重新使用缓冲区及正常析构后的检查均为零。完整 483,268 条 PostGIS 背景上已复现同类信号：分配栈指向 PostgreSQL 编码器的发送缓冲区，运行时读取的 BytesMut 指针减去其编码偏移，精确还原被报告的分配地址；持有该缓冲区的连接任务可从服务根对象追溯。该次复现定性为工具漏识别引用。

仓库提供 [最小诊断程序](../crates/engine/examples/leaks_probe.rs)，依次暂停在“持有已消费数据的缓冲区”“重新使用”“释放”三个阶段，每一步按 Enter 继续：

```sh
cargo build --locked -p geoledger-engine --example leaks_probe
diagnostic_dir="$(mktemp -d)"
cp target/debug/examples/leaks_probe "$diagnostic_dir/leaks_probe"
cat > "$diagnostic_dir/debug.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict><key>com.apple.security.get-task-allow</key><true/></dict></plist>
PLIST
codesign --force --sign - --entitlements "$diagnostic_dir/debug.plist" "$diagnostic_dir/leaks_probe"
MallocStackLogging=1 MallocStackLoggingNoCompact=1 "$diagnostic_dir/leaks_probe"
```

在另一终端对程序打印的 PID 执行 `leaks <PID>`，保存各阶段结果。典型结果依次为 96 KiB、0、0；分配栈指向 `BytesMut::with_capacity`。Rust 对象可继续写入并正常释放，这提供了比单次扫描更完整的所有权证据。

服务排查使用相同方式给独立的 `geoledger-server` 副本添加诊断签名，配合 `benchmark_incremental.py --server /temporary/diagnostic-server` 和完整背景夹具运行；在启动环境中设置上述两个 MallocStackLogging 变量。分配日志会改变运行开销，诊断结果与常规性能数据分别保存。监督脚本会分别记录原生检查的 no_signal、signal、error 状态，出现信号时自动保存内存图；功能成功与内存信号分别报告。也可在服务退出前运行 `leaks -outputGraph /temporary/service.memgraph <PID>` 保存可追溯的内存图。调试权限仅用于临时诊断副本，发布二进制保持原有签名。

# 发布验收

[开发指南](development.md) · [生产运行](production.md)

## 可重复构建

提交到源码的 Go、Python、TypeScript 协议绑定由 `grpcio-tools==1.78.0` 内置的同一 protoc 生成，Go 插件固定为 `protoc-gen-go v1.36.12`、`protoc-gen-go-grpc 1.5.1`，ts-proto 固定为 `2.18.0`。在 Python 虚拟环境安装工具，执行 `PYTHON=<虚拟环境解释器> scripts/generate-sdk.sh`。脚本在写入前核对版本；连续生成两次的文件摘要应一致，CI 检查生成结果与提交完全相同。

容器构建与运行基础镜像使用固定 digest；构建工具来源、SHA256 与版本由 `.security` 锁文件管理。AMD64 与 ARM64 分别在 `ubuntu-24.04`、`ubuntu-24.04-arm` 原生 runner 构建，不使用 QEMU 编译。每个架构的构建阶段与运行阶段分别扫描；合并时保留原始 image、SBOM 和 provenance blobs，再对完整双架构索引扫描并验收。Rust 扫描清单按包名和版本逐项覆盖 `Cargo.lock`，系统包及 npm、编译器清单一并校验。发布直接复制通过检查的 OCI blobs，核验 digest 后签名。Windows 原生依赖按校验和安装并运行真实存储启动测试。

## 候选版本与发布配置

当前候选版本为 `0.3.0-alpha.8`，Python 包使用等价版本 `0.3.0a8`。创建标签前执行 `python3 scripts/check-release-version.py --tag v0.3.0-alpha.8`，将通过检查的提交作为候选 SHA。

### 仅源码与 Docker Hub 的预发布

[Source prerelease and Docker Hub](../.github/workflows/source-docker-release.yml) 是独立的手动入口，仅发布 GitHub prerelease 和 `docker.io/zwishing/geoledger:<版本>`，覆盖 Linux AMD64 / ARM64；不分发 npm/PyPI 包、Go 模块标签或独立二进制。完整 SDK 发布继续使用下文的既有流程。

先将版本准备与流程改动提交至 main，再在 GitHub Actions 中选择该工作流和 main，填写 `source_sha`（当前 main HEAD 的完整 40 位提交 SHA；重跑可使用已固定标签的原候选）及 `release_tag`（例如 `v0.3.0-alpha.8`）。不要先从本地推送该版本 tag，否则会启动完整 SDK 发布链。工作流验证候选属于 main、包版本一致、已有 tag 没有冲突，以及既有 `DOCKERHUB_USERNAME` / `DOCKERHUB_TOKEN` 配置存在；不新增凭据。

预检以 `contents: write` 的内置 `GITHUB_TOKEN` 创建并核对候选 tag，之后才启动同 SHA 完整 CI、[性能验收脚本](../scripts/release-performance.sh)、受检构建阶段和双架构运行镜像、不可变 digest 推送和签名、两架构原生拉取与 SDK 验收，最后公开 prerelease。性能检查保留 20 并发、300 秒和原 p95 门限，使用显式 CRS 的 10,000 条合成要素及一次性数据库。镜像验收检查源码 revision、服务版本、统一容器健康状态、重启后的 API Key 及 SDK 业务流程。

工作流使用仓库 `GITHUB_TOKEN` 创建 tag，避免递归触发完整 SDK 流程。新标签只允许指向预检时的 main HEAD，避免历史工作流差异需要额外 Workflows 写权限；若 main 在预检中前移或权限不足，会在构建与镜像推送前失败。标签固定后 main 可以继续前进，重跑只接受同一 SHA，完成阶段绝不补建或移动标签。失败的验收可能留下候选 tag，但不代表公开 Release，也不会自动删除或复用为其他提交。

Release 查询使用带认证、分页的 releases 列表匹配 `tag_name`，同时识别草稿与公开版本。创建草稿后直接核对创建响应中的 ID 和身份，不依赖列表立即可见；查询失败也不会被当作不存在。已上传附件仍逐项校验摘要，公开版本不补写附件。已有镜像标签指向不同 digest、已有公开 Release、候选 SHA 或验收证据不一致时停止；不会覆盖已有产物。中断后优先重跑同一次运行的失败 job，保留已通过的产物。镜像推送后若原生验收失败，GitHub prerelease 保持未公开；检查失败证据，不把镜像存在当作整个发布完成。

签名由被调用的 `secure-images.yml` 产生。验收精确要求证书身份 `https://github.com/mapseekai/geoledger/.github/workflows/secure-images.yml@refs/heads/main` 和 issuer `https://token.actions.githubusercontent.com`；调用方工作流名称不能代替签名方身份。

### alpha.4 已发布镜像的受控恢复

运行 `38732087093` 的同 SHA CI、性能、两架构原生构建、扫描、Docker Hub 推送和签名已成功，验收因预期签名身份写成调用方而失败。镜像源码候选仍为 `6dccccd5ebc655f520a08c374823cc1389651f81`，保留标签 `0.3.0-alpha.4` 对应的 `sha256:15474870959a39598366f6fb455044e7e5bcd06b222c71e2dd60f54646dd1b3b`。

在 main 手动启动 [Recover alpha.4 source prerelease](../.github/workflows/source-release-recovery.yml)，无需输入参数。该一次性入口核对原运行全部前置 job、固定 artifact ID 和归档 SHA256、来源 SHA 及当前 registry 标签。使用全新的匿名 Docker 配置，在 AMD64 / ARM64 原生 runner 验证精确签名身份并按 digest 拉取，执行健康、版本、源码 revision、重启前后 SDK smoke。两边均成功后再次核对标签，要求指向原候选的 Git tag 已存在，随后创建 GitHub prerelease，附上原性能证据、镜像清单、恢复记录及新 smoke 结果。

恢复流程不重新构建、推送或签名镜像，不发布 SDK，也不使用 registry 凭据。修复流程的 main 提交与镜像源码 SHA 分开记录。不要为这次恢复重跑旧运行（它仍使用错误的预期身份），也不要启动正常构建流程试图覆盖已发布标签。原产物过期、校验不一致或已有公开 Release 时停止，重新评估恢复方案。

当前存储格式仍为 19，现有格式 19 数据库可继续使用。其他存储格式及逻辑导出由当前版本明确拒绝，旧库、备份及匹配的旧版程序独立保留。发布流程不执行业务部署或旧数据迁移。

GitHub 的 `release` environment 配置 `NPM_TOKEN` secret，授予 `@geoledger/client` 的发布权限。PyPI 项目 `geoledger-client` 配置 Trusted Publisher：仓库 `mapseekai/geoledger`、工作流 `release.yml`、environment `release`；完成绑定后将该 environment 的 `PYPI_TRUSTED_PUBLISHING` variable 设置为 `true`。凭据由仓库与包管理员在平台设置中维护。预检验证配置存在，实际发布验证授权生效。

## 发布顺序

候选提交先通过同 SHA 的完整 CI、双后端 SDK、真实数据库 TLS 校验、受限数据库角色原生恢复与浏览器测试。PostGIS 门禁必须执行两个原先可选的 TLS 测试，并在 verify-full 下完成恢复后业务验证。npm 与 PyPI 的发布配置在构建交付前检查。二进制和镜像完成后创建草稿 Release，验证统一 combined 镜像清单属于当前 SHA；SDK 分发成功后才公开 Release。npm 包的 `repository` 指向实际源码仓库及 `sdk/ts` 目录，与 provenance 的发布来源一致。Rust 客户端按同一 tag 从 Git 引用。

发布中断后使用同一次运行的失败 job 重跑，保留已经验收的构建产物。草稿 Release 校验标签提交和已有资产 SHA256，补传缺失资产；npm/PyPI 校验已有包的文件内容与权限（忽略归档时间及压缩差异），仅发布缺失包；Go 标签必须解析到候选 SHA。已有内容不一致时流程停止，保留原产物用于排查。SDK 安装验收连接打包后的服务器，执行新增、修改、删除、历史读取及发布重试，成功后公开 Release。

镜像复审读取 Release 中的不可变 digest。已有发布缺失清单时，应从原发布的签名、OCI revision 和构建 provenance 恢复准确清单，再重新扫描；该证据齐全前保留失败状态。新的候选版本通过正常流程生成完整清单。原始历史产物的缺失不能用当前浮动标签补造。

发布前记录目标平台、版本与存储格式、镜像 digest、SDK 安装检查和测试结果。每个发布包在干净环境安装，连接同版本服务完成新增、修改、删除和历史查询。

## 数据恢复与权限

`python3 scripts/test-postgis-release.py --output <新的结果目录>` 在一次性 Docker 数据库中验证：非超级用户跨 schema 纳管、保存发布、原生 pg_dump/pg_restore、历史与审计一致性、成员凭据恢复、恢复后继续发布、1/100/500 条原表逐项校验、解绑保留原表。加 `--web --chromium <浏览器路径>` 验证真实控制台纳管与编辑。脚本输出实测恢复耗时，结束时清理容器。备份文件与日志仅保留在指定测试目录。

生产使用独立初始化账号与运行账号。目标业务环境按正式备份体量演练恢复，记录允许丢失的数据窗口、恢复用时、凭据恢复和告警通知结果。

## 性能与发布窗口

使用当前候选 release 二进制执行 [增量性能验证](performance.md)。已有数据的 10、100、500 条增删改分别记录保存与发布延迟、p95/p99、错误率、内存与连接增长。目标硬件上完成持续负载和故障恢复验证后确定容量与超时配置。历史测试保留其版本和机器条件。

2026-10-11 的格式 13 代码通过全仓库检查和双后端四语言 SDK 联调；同日上一候选已完成浏览器流程、受限角色和原生恢复演练。当前 `0.3.0-alpha.2` 候选在 48 万条背景、20 路并发下完成 404 个增删改循环且零错误，原表逐项校验及背景总数核对通过。首次发布总体 p95 为 5.92 秒，最慢批量分组 p95 为 8.96 秒，超过 5 秒门限；宿主机存在持续大量换页，保留修复和结果，待空闲环境复测验收。具体测量见性能指南。正式发布还需当前提交的远程 CI、完整产物清单及目标环境验收结果。

控制台首版采用单实例，发布维护窗口内更新，用户在重启后重新登录；服务端发布收据用于确认在途操作。跨副本持续登录作为独立部署能力验收。

## 持续增量负载

`benchmark_incremental.py --load-seconds 300 --load-workers 20` 在同一已有背景数据集上执行并发新增、修改、删除。每个并发客户端轮换 10/100/500 条，保存分批最多 100 条，每次发布还重放原请求确认幂等性；报告输出 p95/p99、错误和完整循环数，默认保存、首次发布及收据重放分别计算 p95，门限为 5 秒。该门限用于发现退化，业务响应目标应按实际用户期望收紧。监督器同时采样服务与客户端 RSS，并在测试后核对原表与版本库要素总数；延迟门限超标时仍执行完整性检查，分别保留两类结果。夜间工作流对两个后端执行该检查，真实业务几何使用同一入口单独验收。

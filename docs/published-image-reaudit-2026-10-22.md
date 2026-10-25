# 已发布镜像复审：2026-10-22

## 结论与证据

2026-10-22 05:58（北京时间）的[失败工作流](https://github.com/mapseekai/geoledger/actions/runs/39682973379)发生了版本基线错位：当前 main 的审计脚本将 main 的 `Cargo.lock` 用于校验旧发布镜像。两个架构均未发生本次报错所指的依赖记录缺失；Trivy 对镜像锁文件的识别也没有遗漏。

- 审计代码基线：`f34249eca825e730e3626cdb056c73aa903538c4`。
- 发布版本：[v0.3.0-alpha.6](https://github.com/mapseekai/geoledger/releases/tag/v0.3.0-alpha.6)。
- 发布源提交：`ffa3ecb50305016862f6b34ea648c724eb9937b7`。
- 发布清单中的镜像：`docker.io/zwishing/geoledger@sha256:94d937c75a298d31b983e5e5bb236468cb231c797b982d71f59b57616bb8b035`。
- 失败产物：`published-image-scan`，Artifact ID `12384627850`；下载 ZIP SHA256：`5c52a29ea09b7cd88e72528a5a9693268cd73b1cb76a89960a5c50879ac48efe`，与 Actions 日志一致。

| 对比对象 | Cargo 包数 | rustls-webpki 版本 | 相对发布锁文件缺失记录 |
| --- | ---: | --- | ---: |
| 发布 SHA 的 Cargo.lock | 347 | 0.103.15 | 0 |
| main 的 Cargo.lock | 347 | 0.103.16 | 0.103.15 被替换为 0.103.16 |
| 失败产物 amd64 扫描报告 | 347 | 0.103.15 | 0 |
| 失败产物 arm64 扫描报告 | 347 | 0.103.15 | 0 |

两个报告相对 main 均仅“缺少” `rustls-webpki@0.103.16`，相对发布版本则包名与版本集合完全一致。main 提交 `f34249e` 仅替换了该 Cargo 依赖的版本与校验和。复审不应要求历史镜像具有未来提交中的依赖版本；历史版本的漏洞风险继续由当前漏洞扫描策略判断。

从上述 digest 实际拉取全部 OCI manifest 后，核对了配置中的 revision 标签、两个平台 manifest 和原始镜像层内 `usr/share/geoledger/Cargo.lock`。两个锁文件均与发布提交逐字节相同，SHA256 为 `61a57051e5077061d582a15350177ec67f8b06cd6c9315e0305565f5ca80b121`。通过 GitHub Contents API 按发布 SHA 获取的锁文件也具有相同 SHA256。main 锁文件 SHA256 为 `f36f507d30e703c0e6ae2e2f4667dcc08dfba6d3bc2a55e64bdfeca23ded51d7`。

| 平台 | manifest digest | image config digest |
| --- | --- | --- |
| linux/amd64 | `sha256:d5ab8992c426d6add4b7faa25e60238c012ed9469d1363933bb22d36f5ad04de` | `sha256:2a6ae160d3b3b9e3b3a4a1cfcb81d570467ac7fe6f52f911c3abe4dbaa41e2a5` |
| linux/arm64 | `sha256:9557b0aa948ee0a2895b2ea562bbb8bf450257d94d9b31f47fe785e7dd57ac4c` | `sha256:5fd95717fa0733367fca1149ef486ccfa4432424e1ff63d29724ebea10856b89` |

## 修复

1. 发布标签解析为完整 SHA，并继续要求发布清单的 source SHA 与之相等。
2. 从该 SHA 获取 `Cargo.lock`，显式传给依赖完整性校验，不回退到 main。
3. 保留当前审计代码、固定扫描器版本、实时漏洞库、OS/语言依赖检查、双架构、revision 和扫描配置身份校验、不可变 digest 及 HIGH/CRITICAL（含未修补公告）阻断规则。
4. 扫描凭据记录基线锁文件 SHA256，复核时拒绝替换后的锁文件；复审在扫描完成后执行 `verify()`。
5. 诊断产物保存发布锁文件、源 SHA、digest、逐架构扫描报告和明确的审计失败原因。缺失依赖报错列出包名与版本。

未更改镜像、Cargo.lock、漏洞阈值或忽略规则。修复解决依赖基线选择，不替代发布新版本来更新历史镜像中的依赖。

## 实测结果与边界

本地 Linux x86_64 环境使用 Skopeo 1.13.3 和仓库校验和固定的 Trivy 0.75.0，实际拉取上述 digest，扫描两个平台并验证扫描凭据。ARM64 使用其独立 OCI manifest 做静态扫描；没有运行 ARM64 容器程序。

2026-10-22 11:32（北京时间），两个架构的 `scan()` 和 `verify()` 均通过。每个平台识别到 Alpine 包 40 个、Node 包 36 个、Cargo 包 347 个；Cargo 相对发布锁文件缺失 0 个，HIGH/CRITICAL 0 条。此次 Trivy DB 元数据的 UpdatedAt 为 `2026-10-22T01:15:55.263770788Z`，DownloadedAt 为 `2026-10-22T03:32:09.572148154Z`，未使用离线替代数据库。

每个架构另有 `CVE-2026-107891` 的两条 UNKNOWN 记录，分别对应 `musl` 和 `musl-utils` `1.2.6-r2`；现有策略仅阻断 HIGH/CRITICAL，因此通过不代表零公告。此处仅记录扫描器输出，未独立判定该公告的可利用性。

| 检查 | 结果 |
| --- | --- |
| 原始双架构报告 + main 锁文件 | 精确复现 rustls-webpki@0.103.16 缺失 |
| 原始双架构报告 + 发布锁文件 | 通过；依赖集合完全相同 |
| 实际镜像层内锁文件 + 发布源文件 | 两个平台均逐字节相同 |
| 同 digest 重新拉取、双架构 scan + verify | 通过 |
| `python3 -m unittest discover -s scripts -p test_supply_chain.py` | 83 项通过；含历史基线、基线篡改、真实缺包、高危漏洞及复审编排回归 |
| `python3 -m unittest discover -s scripts -p test_source_release.py` | 18 项通过 |
| `check-docs.py`、`check-env.py`、`test_container.py`、`check-release-policy.py` | 通过；容器静态策略测试 6 项通过 |
| `test_deployment_config.py` | 跳过；环境缺少 Docker Compose |
| Python 语法编译、`git diff --check` | 通过 |
| `./scripts/check.sh` | 首项因缺少 ast-grep 0.51.0 停止；此环境亦未安装 Cargo，未完成全仓 Rust 检查 |
| GitHub 原工作流重新执行 | 未执行；本地 gh 未认证，当前连接未提供 workflow_dispatch 操作。已通过已认证 GitHub 连接读取原产物和日志；实测镜像门禁使用真实拉取及实时扫描，不是扫描器模拟测试 |

## 复现

在有 GitHub CLI 认证、Skopeo 和 Trivy 运行条件的环境，从包含修复的检出运行：

```bash
export GEOLEDGER_AUDIT_DIR="$(mktemp -d)"
python3 scripts/install-trivy.py --destination "$GEOLEDGER_AUDIT_DIR/scanners"
export PATH="$GEOLEDGER_AUDIT_DIR/scanners:$PATH"
python3 scripts/audit-published-images.py \
  --repository mapseekai/geoledger \
  --tag v0.3.0-alpha.6 \
  --output "$GEOLEDGER_AUDIT_DIR/published-reports"
```

该入口会下载发布清单、按发布 SHA 读取源锁文件、按原始 digest 拉取并核验全部 OCI 内容、扫描两个架构、执行凭据复核。预期 `audit-status.json` 中的 `source_sha` 为发布 SHA，两个平台均覆盖，依赖基线 hash 与上文一致。漏洞库随时间更新，未来新增 HIGH/CRITICAL 时应继续失败，不能将本次通过视为永久结论。

独立复现原错误与修复后的依赖集合校验：

```bash
export GEOLEDGER_AUDIT_DIR="$(mktemp -d)"
gh run download 39682973379 --repo mapseekai/geoledger \
  --name published-image-scan --dir "$GEOLEDGER_AUDIT_DIR/original-reports"
git show ffa3ecb50305016862f6b34ea648c724eb9937b7:Cargo.lock \
  > "$GEOLEDGER_AUDIT_DIR/release-Cargo.lock"
python3 - <<'PY'
import importlib.util
import json
import os
from pathlib import Path

spec = importlib.util.spec_from_file_location('gate', 'scripts/image-gate.py')
gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate)
root = Path(os.environ['GEOLEDGER_AUDIT_DIR'])
release_lock = root / 'release-Cargo.lock'
for report in (root / 'original-reports/combined').glob('*.json'):
    value = json.loads(report.read_text())
    try:
        gate.validate_source_inventory(value, 'combined')
    except ValueError as error:
        print(report.name, error)
    gate.validate_source_inventory(value, 'combined', release_lock)
    print(report.name, 'PASS release baseline')
PY
```
